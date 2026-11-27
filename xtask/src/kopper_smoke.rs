//! `cargo xtask kopper-smoke [--prefix <dir>] [--kicad-cli <file>]`: proves an
//! installed 1 Kopper package works, end to end, on a public board.
//!
//! 1. The package is one (`kopper_package::is_package`), and its two binaries
//!    still have the SHA-256 values its manifest records.
//! 2. In a fresh folder under `target/kopper-smoke/`, it starts the package's
//!    `konnect` by pipe. That folder serves as the scratch `HOME`, `TMPDIR`,
//!    `KONNECT_STATE_DIR` and `KICAD_CONFIG_HOME`. The environment is otherwise
//!    empty, so no `KONNECT_FASTROUTE` and no user config apply. A `konnect.toml`
//!    names only `kicad-cli`, so simulation stays at its default, off.
//! 3. Simulation is absent: no simulation tool in `tools/list`, no `simulation`
//!    toolbox, and no ngspice runtime.
//! 4. `route_specctra_dsn` at effort `fast`, with no `fastroute_path`, routes the
//!    tracked export of the public `specctra_pours_routing` fixture. That export is
//!    kept current by `the_kopper_smoke_export_matches_a_fresh_export`. The call
//!    must use the package's own fastroute (`bundled`, at its path, with its
//!    hash), write a session and pass every judge gate.
//! 5. The installed Konnect and the owner's KiCad settings are unchanged.
//!
//! It writes only under `target/kopper-smoke/`. It runs fastroute only through
//! Konnect, which isolates it with `sandbox-exec`; for that reason this task is
//! not itself run under a sandbox, since macOS refuses a nested one.

use crate::kopper_package::{self, MANIFEST};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// The public fixture the smoke routes, under `crates/konnect-core/tests/fixtures/`.
const BOARD: &str = "specctra_pours_routing";

/// The macOS kicad-cli.
const KICAD_CLI: &str = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli";

/// The installed Konnect, under the account's home; the smoke fails if it changes.
const INSTALLED_KONNECT: &str = ".local/share/konnect/bin/konnect";

/// The owner's KiCad settings, under the account's home: every regular file below
/// it, whatever the KiCad version, is hashed before and after.
const KICAD_SETTINGS: &str = "Library/Preferences/kicad";

/// One verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub pass: bool,
    pub detail: String,
}

fn check(name: &'static str, pass: bool, detail: impl Into<String>) -> Check {
    Check {
        name,
        pass,
        detail: detail.into(),
    }
}

/// `cargo xtask kopper-smoke` (see the module text).
pub fn run(args: &[String], repo: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    let mut prefix = home.join(kopper_package::DEFAULT_PREFIX);
    let mut kicad_cli = PathBuf::from(KICAD_CLI);
    let mut index = 0;
    while index < args.len() {
        let value = args
            .get(index + 1)
            .map(PathBuf::from)
            .ok_or_else(|| format!("{} needs a path", args[index]))?;
        match args[index].as_str() {
            "--prefix" => prefix = value,
            "--kicad-cli" => kicad_cli = value,
            other => {
                return Err(format!(
                    "unknown argument {other}; usage: cargo xtask kopper-smoke [--prefix <dir>] [--kicad-cli <file>]"
                ))
            }
        }
        index += 2;
    }
    let account = crate::kopper_package::account_home().unwrap_or(home);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let out = repo
        .join("target/kopper-smoke")
        .join(format!("{stamp}-{}", std::process::id()));
    if kopper_package::present(&out) {
        return Err(format!("{} exists already", out.display()));
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("create {}: {e}", out.display()))?;
    let checks = smoke(&Smoke {
        prefix,
        kicad_cli,
        fixtures: repo.join("crates/konnect-core/tests/fixtures"),
        out: out.clone(),
        account,
    });
    for check in &checks {
        println!(
            "{} {}: {}",
            if check.pass { "PASS" } else { "FAIL" },
            check.name,
            check.detail
        );
    }
    let summary = json!(checks
        .iter()
        .map(|c| json!({"check": c.name, "pass": c.pass, "detail": c.detail}))
        .collect::<Vec<_>>());
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_string_pretty(&summary).expect("a JSON value prints"),
    )
    .map_err(|e| format!("write the summary in {}: {e}", out.display()))?;
    let failed = checks.iter().filter(|check| !check.pass).count();
    println!("Record: {}", out.display());
    if failed == 0 {
        println!("1 Kopper smoke: all {} checks passed.", checks.len());
        Ok(())
    } else {
        Err(format!(
            "1 Kopper smoke: {failed} of {} checks failed",
            checks.len()
        ))
    }
}

pub struct Smoke {
    pub prefix: PathBuf,
    pub kicad_cli: PathBuf,
    pub fixtures: PathBuf,
    pub out: PathBuf,
    /// The account's home, where the protected files are.
    pub account: PathBuf,
}

/// Every check, in order. A step that cannot run is a failed check, and the
/// later steps that depend on it are not attempted.
pub fn smoke(smoke: &Smoke) -> Vec<Check> {
    let protected_before = protected_hashes(&smoke.account);
    let mut checks = package_checks(&smoke.prefix);
    if checks.iter().any(|check| !check.pass) {
        return checks;
    }
    let staged = match stage(&smoke.fixtures, &smoke.out, &smoke.kicad_cli) {
        Ok(staged) => staged,
        Err(error) => {
            checks.push(check("stage", false, error));
            return checks;
        }
    };
    if let Err(error) = session_checks(smoke, &staged, &mut checks) {
        checks.push(check("session", false, error));
    }
    checks.push(protected_check(
        &protected_before,
        &protected_hashes(&smoke.account),
    ));
    checks
}

/// The protected files' hashes, by path relative to the account's home: the
/// installed Konnect and every regular file under the KiCad settings.
pub fn protected_hashes(account: &Path) -> std::collections::BTreeMap<String, String> {
    let mut hashes = std::collections::BTreeMap::new();
    let mut stack = vec![account.join(KICAD_SETTINGS)];
    let mut files = vec![account.join(INSTALLED_KONNECT)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                Ok(kind) if kind.is_file() => files.push(entry.path()),
                _ => {}
            }
        }
    }
    for file in files {
        if let Ok(sha) = kopper_package::sha256_of(&file) {
            let key = file.strip_prefix(account).unwrap_or(&file);
            hashes.insert(key.to_string_lossy().into_owned(), sha);
        }
    }
    hashes
}

/// Nothing protected changed, appeared or went away, and the installed Konnect
/// was there to be checked.
pub fn protected_check(
    before: &std::collections::BTreeMap<String, String>,
    after: &std::collections::BTreeMap<String, String>,
) -> Check {
    let changed: Vec<&str> = before
        .keys()
        .chain(after.keys())
        .filter(|key| before.get(*key) != after.get(*key))
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let installed = before.contains_key(INSTALLED_KONNECT);
    check(
        "protected files unchanged",
        installed && changed.is_empty(),
        if !installed {
            format!("the installed Konnect ({INSTALLED_KONNECT}) was not found to check")
        } else if changed.is_empty() {
            format!(
                "the installed Konnect and {} KiCad settings files",
                before.len() - 1
            )
        } else {
            format!("changed: {}", changed.join(", "))
        },
    )
}

/// The package is one, and both binaries match its manifest.
pub fn package_checks(prefix: &Path) -> Vec<Check> {
    if !kopper_package::is_package(prefix) {
        return vec![check(
            "package",
            false,
            format!(
                "{} is not a 1 Kopper package; run cargo xtask kopper-install",
                prefix.display()
            ),
        )];
    }
    let manifest: Value = std::fs::read_to_string(prefix.join(MANIFEST))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    let mut checks = vec![check("package", true, prefix.display().to_string())];
    for name in ["konnect", "fastroute"] {
        let path = prefix.join("bin").join(name);
        let expected = manifest[name]["sha256"].as_str().unwrap_or_default();
        let actual = kopper_package::sha256_of(&path).unwrap_or_else(|error| error);
        checks.push(check(
            if name == "konnect" {
                "konnect matches the manifest"
            } else {
                "fastroute matches the manifest"
            },
            !expected.is_empty() && actual == expected,
            format!("{}: {actual}", path.display()),
        ));
    }
    checks
}

/// What [`stage`] laid out.
pub struct Staged {
    pub dir: PathBuf,
    pub dsn: PathBuf,
    pub ses: PathBuf,
}

/// Copies the fixture's board, project, DSN and manifest into `out/board/`,
/// points the manifest at that board copy, and writes `out/konnect.toml`.
pub fn stage(fixtures: &Path, out: &Path, kicad_cli: &Path) -> Result<Staged, String> {
    let board_dir = out.join("board");
    for dir in ["home", "tmp", "state", "kicad-config", "board"] {
        std::fs::create_dir_all(out.join(dir))
            .map_err(|e| format!("create {}: {e}", out.join(dir).display()))?;
    }
    for suffix in [".kicad_pcb", ".kicad_pro", ".dsn", ".dsn.konnect.json"] {
        let name = format!("{BOARD}{suffix}");
        std::fs::copy(fixtures.join(&name), board_dir.join(&name))
            .map_err(|e| format!("copy {name}: {e}"))?;
    }
    let board = board_dir
        .join(format!("{BOARD}.kicad_pcb"))
        .canonicalize()
        .map_err(|e| format!("resolve the board copy: {e}"))?;
    let manifest_path = board_dir.join(format!("{BOARD}.dsn.konnect.json"));
    let mut manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).map_err(|e| format!("read the manifest: {e}"))?,
    )
    .map_err(|e| format!("the manifest is not JSON: {e}"))?;
    manifest["board_path"] = json!(board.to_string_lossy());
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("a JSON value prints"),
    )
    .map_err(|e| format!("write the manifest: {e}"))?;
    let config = format!(
        "kicad_cli = \"{}\"\n",
        kicad_cli
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    );
    std::fs::write(out.join("konnect.toml"), config)
        .map_err(|e| format!("write konnect.toml: {e}"))?;
    Ok(Staged {
        dir: out.to_path_buf(),
        dsn: board_dir.join(format!("{BOARD}.dsn")),
        ses: board_dir.join(format!("{BOARD}.ses")),
    })
}

/// One `konnect` by pipe: requests out, answers back by id, each with a deadline.
struct Session {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: mpsc::Receiver<String>,
    next: u64,
}

impl Session {
    fn start(exe: &Path, staged: &Staged) -> Result<Self, String> {
        let dir = &staged.dir;
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new(exe)
            // Its own process group, so stopping it also stops the sandboxed
            // fastroute runs it started.
            .process_group(0)
            .current_dir(dir)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", dir.join("home"))
            .env("TMPDIR", dir.join("tmp"))
            .env("KONNECT_STATE_DIR", dir.join("state"))
            .env("KICAD_CONFIG_HOME", dir.join("kicad-config"))
            .env("KONNECT_CALL_LOG", "off")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(
                std::fs::File::create(dir.join("konnect.stderr.log"))
                    .map_err(|e| format!("create the stderr log: {e}"))?,
            )
            .spawn()
            .map_err(|e| format!("start {}: {e}", exe.display()))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines,
            next: 0,
        })
    }

    fn request(&mut self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{message}").map_err(|e| format!("send {method}: {e}"))?;
        let deadline = std::time::Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .map_err(|_| format!("{method}: no answer within {} s", wait.as_secs()))?;
            let Ok(answer) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if answer["id"] == json!(id) && answer.get("method").is_none() {
                if let Some(error) = answer.get("error") {
                    return Err(format!("{method}: {error}"));
                }
                return Ok(answer["result"].clone());
            }
        }
    }

    fn notify(&mut self, method: &str) -> Result<(), String> {
        let message = json!({"jsonrpc": "2.0", "method": method});
        writeln!(self.stdin, "{message}").map_err(|e| format!("send {method}: {e}"))
    }

    /// A tool call: `(is_error, body)`, the body being the JSON text when it is JSON.
    fn tool(
        &mut self,
        name: &str,
        arguments: Value,
        wait: Duration,
    ) -> Result<(bool, Value), String> {
        let result = self.request(
            "tools/call",
            json!({"name": name, "arguments": arguments}),
            wait,
        )?;
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        let body = serde_json::from_str(text).unwrap_or_else(|_| json!({"text": text}));
        Ok((result["isError"] == json!(true), body))
    }
}

impl Drop for Session {
    /// Stops the server and everything it started: SIGTERM to its process group,
    /// up to 10 s for a clean exit, then SIGKILL to the group.
    fn drop(&mut self) {
        let group = self.child.id() as libc::pid_t;
        // SAFETY: plain signal sends to the group this session created.
        unsafe { libc::killpg(group, libc::SIGTERM) };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // SAFETY: as above; the group may already be gone.
        unsafe { libc::killpg(group, libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

/// The checks through the server, pushed as they are made, so a later failure
/// keeps the earlier verdicts.
fn session_checks(smoke: &Smoke, staged: &Staged, checks: &mut Vec<Check>) -> Result<(), String> {
    let exe = smoke.prefix.join("bin/konnect");
    let mut session = Session::start(&exe, staged)?;
    let short = Duration::from_secs(60);
    session.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {},
               "clientInfo": {"name": "kopper-smoke", "version": "1"}}),
        short,
    )?;
    session.notify("notifications/initialized")?;
    let (_, toolboxes) = session.tool("list_toolboxes", json!({}), short)?;
    checks.push(toolboxes_check(&toolboxes));
    let (_, info) = session.tool("get_installation_info", json!({}), short)?;
    checks.extend(installation_checks(&info, &exe));
    session.tool("load_toolset", json!({"name": "integration"}), short)?;
    let started = std::time::Instant::now();
    let (is_error, body) = session.tool(
        "route_specctra_dsn",
        // Row BS: the smoke check routes the public board with no KiCad open.
        json!({"dsn_path": staged.dsn, "ses_output_path": staged.ses, "effort": "fast",
               "require_live_board": false}),
        Duration::from_secs(1800),
    )?;
    let _ = std::fs::write(
        staged.dir.join("route-response.json"),
        serde_json::to_string_pretty(&body).expect("a JSON value prints"),
    );
    let manifest: Value = std::fs::read_to_string(smoke.prefix.join(MANIFEST))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    checks.extend(route_checks(
        is_error,
        &body,
        &smoke.prefix,
        &manifest,
        &staged.ses,
        started.elapsed(),
    ));
    let list = session.request("tools/list", json!({}), short)?;
    checks.push(tools_list_check(&list));
    Ok(())
}

/// No simulation toolbox is listed.
pub fn toolboxes_check(body: &Value) -> Check {
    let names: Vec<&str> = body["toolsets"]
        .as_array()
        .map(|sets| sets.iter().filter_map(|set| set["name"].as_str()).collect())
        .unwrap_or_default();
    check(
        "no simulation toolbox",
        !names.is_empty() && !names.contains(&"simulation"),
        format!("{} toolboxes listed", names.len()),
    )
}

/// No simulation tool is listed (after the route call loaded `integration`).
pub fn tools_list_check(list: &Value) -> Check {
    let simulation: Vec<&str> = konnect_core::router::registry::tools_for("simulation")
        .map(|tools| tools.iter().map(|tool| tool.name).collect())
        .unwrap_or_default();
    let listed: Vec<&str> = list["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    let found: Vec<&str> = listed
        .iter()
        .copied()
        .filter(|name| simulation.contains(name))
        .collect();
    check(
        "no simulation tool in tools/list",
        !listed.is_empty() && !simulation.is_empty() && found.is_empty(),
        if found.is_empty() {
            format!("{} tools listed", listed.len())
        } else {
            format!("listed: {}", found.join(", "))
        },
    )
}

/// The server is the package's `konnect`, and carries no simulation runtime.
pub fn installation_checks(info: &Value, exe: &Path) -> Vec<Check> {
    let running = info["runtime"]["executable_path"]
        .as_str()
        .map(PathBuf::from)
        .and_then(|path| path.canonicalize().ok());
    let expected = exe.canonicalize().ok();
    let ngspice = info["ngspice"]["probe_status"].as_str().unwrap_or("(none)");
    vec![
        check(
            "the server is the package's konnect",
            running.is_some() && running == expected,
            format!("{running:?}"),
        ),
        check(
            "no simulation runtime",
            ngspice == "not_available",
            format!("ngspice.probe_status {ngspice}"),
        ),
    ]
}

/// The route used the package's fastroute, wrote a session and passed the judge.
pub fn route_checks(
    is_error: bool,
    body: &Value,
    prefix: &Path,
    manifest: &Value,
    ses: &Path,
    elapsed: Duration,
) -> Vec<Check> {
    let engine = &body["engine"];
    let bundled = prefix.join("bin/fastroute").canonicalize().ok();
    let path = engine["path"]
        .as_str()
        .map(PathBuf::from)
        .and_then(|path| path.canonicalize().ok());
    let gates = &body["completion"]["judged"]["all_gates_pass"];
    let unconnected = &body["completion"]["judged"]["unconnected"];
    vec![
        check(
            "route_specctra_dsn succeeded",
            !is_error && body["success"] != json!(false),
            format!(
                "status {}, {} s",
                body["status"].as_str().unwrap_or("(none)"),
                elapsed.as_secs()
            ),
        ),
        check(
            "it routed with the package's own fastroute",
            engine["name"] == "fastroute"
                && engine["source"] == "bundled"
                && path.is_some()
                && path == bundled
                && engine["sha256"] == manifest["fastroute"]["sha256"],
            format!(
                "source {}, sha256 {}",
                engine["source"].as_str().unwrap_or("(none)"),
                engine["sha256"].as_str().unwrap_or("(none)")
            ),
        ),
        check(
            "a session was written",
            std::fs::metadata(ses).is_ok_and(|meta| meta.is_file() && meta.len() > 0),
            ses.display().to_string(),
        ),
        check(
            "the winner passed every judge gate, with nothing unconnected",
            *gates == json!(true) && *unconnected == json!(0),
            format!("all_gates_pass {gates}, unconnected {unconnected}"),
        ),
        check(
            "the status is complete",
            body["status"] == "complete",
            format!("status {}", body["status"].as_str().unwrap_or("(none)")),
        ),
    ]
}

#[cfg(test)]
#[path = "kopper_smoke_tests.rs"]
mod tests;
