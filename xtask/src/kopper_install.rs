//! `cargo xtask kopper-install`: 1 Kopper in one command.
//!
//! Before anything is written or run, it reads and checks everything it will need,
//! and refuses on anything it cannot handle:
//! - Claude's user `konnect` entry in `~/.claude.json`, which must be a stdio entry
//!   with a `command`;
//! - Codex's `[mcp_servers.konnect]` `command` in `$CODEX_HOME/config.toml` (default
//!   `~/.codex`), which must be one plain quoted string on its own line;
//! - the way-back record `<prefix>.rollback.json`;
//! - that `claude` can be found;
//! - the package arguments.
//!
//! Then it:
//! 1. packages Konnect and the pinned fastroute (`kopper_package`);
//! 2. reads both entries again. Each entry that does not already run a 1 Kopper
//!    package is added to the record, mode 0600, but only when the record does
//!    not hold that client yet: the first original is never overwritten;
//! 3. runs the package's `konnect init` for Claude, and for Codex when Codex has
//!    an entry;
//! 4. points both entries at `<prefix>/bin/konnect`. Claude goes through `claude mcp
//!    remove` and `claude mcp add-json`, and a failed add puts the earlier entry
//!    back. For Codex it rewrites that one `command` line in place. The file's
//!    mode and any link to it are kept, and the file as it was before the first
//!    switch is kept once as `config.toml.before-1-kopper`.
//!
//! `cargo xtask kopper-install --rollback [--prefix <dir>]`:
//! 1. puts every recorded entry back;
//! 2. then runs each recorded binary's `init`. An entry that failed does not stop
//!    the others;
//! 3. when everything worked, it sets the record aside as
//!    `<prefix>.rollback.json.used`.
//!
//! Neither form writes the installed Konnect. Rollback only runs it, for `init`.
//! A new client session picks up the change.

use crate::kopper_package::{self, MANIFEST};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The MCP server name both clients use.
const SERVER: &str = "konnect";

/// Where the outside world is reached, so tests stand in for it.
pub trait Host {
    fn home(&self) -> &Path;
    /// An environment variable of the run.
    fn var(&self, name: &str) -> Option<String>;
    /// Runs `program` with `args` in `cwd`; an error carries what it printed.
    fn run(&mut self, program: &Path, args: &[String], cwd: &Path) -> Result<(), String>;
}

/// The real machine.
pub struct RealHost {
    home: PathBuf,
}

impl Host for RealHost {
    fn home(&self) -> &Path {
        &self.home
    }

    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }

    fn run(&mut self, program: &Path, args: &[String], cwd: &Path) -> Result<(), String> {
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(cwd)
            .env("KONNECT_CALL_LOG", "off")
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|error| format!("start {}: {error}", program.display()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{} {} failed ({}): {} {}",
                program.display(),
                shown(args),
                output.status,
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

/// `args` for a message, with JSON arguments (an entry's `env` may hold secrets)
/// left out.
fn shown(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.starts_with('{') {
                "<entry JSON>"
            } else {
                arg.as_str()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `cargo xtask kopper-install [--rollback [--prefix <dir>] | <kopper-package arguments>]`.
pub fn run(args: &[String], repo: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    let mut host = RealHost { home };
    if args.first().map(String::as_str) == Some("--rollback") {
        let prefix = match &args[1..] {
            [] => host.home.join(kopper_package::DEFAULT_PREFIX),
            [flag, prefix] if flag == "--prefix" => PathBuf::from(prefix),
            _ => {
                return Err("usage: cargo xtask kopper-install --rollback [--prefix <dir>]".into())
            }
        };
        let done = rollback(&mut host, &prefix)?;
        println!("{done}\nStart a new client session to use it.");
        return Ok(());
    }
    // Everything that can refuse, before the build and before any write.
    let prefix = args
        .iter()
        .position(|arg| arg == "--prefix")
        .and_then(|pos| args.get(pos + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home.join(kopper_package::DEFAULT_PREFIX));
    preflight(&host, &prefix)?;
    let request = kopper_package::request_from_args(args, repo)?;
    let packaged = kopper_package::package(&request)?;
    kopper_package::report(&packaged);
    let done = activate(&mut host, &packaged.prefix)?;
    println!("{done}\nStart a new client session to use 1 Kopper.");
    Ok(())
}

/// The checks [`activate`] would fail on, made before anything is written.
pub fn preflight(host: &dyn Host, prefix: &Path) -> Result<(), String> {
    Clients::read(host)?;
    read_record(&rollback_file(prefix))?;
    if host.var("CLAUDE_CONFIG_DIR").is_some() {
        return Err("CLAUDE_CONFIG_DIR is set, and this install reads only ~/.claude.json; nothing was changed".into());
    }
    let path = host.var("PATH").unwrap_or_default();
    if !std::env::split_paths(&path).any(|dir| dir.join("claude").is_file()) {
        return Err("`claude` is not on PATH, so Claude's konnect entry cannot be switched; nothing was changed".into());
    }
    Ok(())
}

/// The two clients' current `konnect` entries.
#[derive(Debug, Clone, PartialEq)]
pub struct Clients {
    /// Claude's user-scope stdio entry, as `~/.claude.json` holds it.
    pub claude: Option<Value>,
    /// The `command` of Codex's `[mcp_servers.konnect]`.
    pub codex: Option<String>,
}

impl Clients {
    /// Both entries; refuses one this install cannot handle.
    pub fn read(host: &dyn Host) -> Result<Self, String> {
        Ok(Self {
            claude: read_claude(host.home())?,
            codex: read_codex(&codex_config(host))?,
        })
    }
}

fn read_claude(home: &Path) -> Result<Option<Value>, String> {
    let path = home.join(".claude.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };
    let entry = serde_json::from_str::<Value>(&text)
        .map_err(|e| format!("{} is not JSON ({e}); nothing was changed", path.display()))?
        .get("mcpServers")
        .and_then(|servers| servers.get(SERVER))
        .cloned();
    if let Some(entry) = &entry {
        let stdio = entry
            .get("type")
            .is_none_or(|kind| kind.as_str() == Some("stdio"));
        if !entry.is_object() || !stdio || !entry["command"].is_string() {
            return Err(format!(
                "Claude's konnect entry in {} is not a stdio entry with a command, which this install does not switch; nothing was changed",
                path.display()
            ));
        }
    }
    Ok(entry)
}

fn read_codex(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => codex_command(&text)
            .map(|found| found.map(|(_, command)| command))
            .map_err(|e| format!("{}: {e}; nothing was changed", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

fn codex_config(host: &dyn Host) -> PathBuf {
    host.var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".codex"))
        .join("config.toml")
}

/// `<prefix>.rollback.json`, beside the package so repackaging keeps it.
pub fn rollback_file(prefix: &Path) -> PathBuf {
    let name = prefix
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    prefix.with_file_name(format!("{name}.rollback.json"))
}

/// The record: an object, `{}` when there is none.
fn read_record(path: &Path) -> Result<serde_json::Map<String, Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Err(format!(
                "{} is not a JSON object; look at it before running again (nothing was changed)",
                path.display()
            )),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::Map::new()),
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

fn write_record(path: &Path, record: &serde_json::Map<String, Value>) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let text = serde_json::to_string_pretty(record).expect("a JSON value prints") + "\n";
    let staged = path.with_extension("json.new");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged)
        .map_err(|e| format!("write {}: {e}", staged.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("write {}: {e}", staged.display()))?;
    std::fs::rename(&staged, path).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Whether `command` runs the Konnect of a 1 Kopper package, or did: a package
/// that has since been removed still counts, by the `<…1-kopper…>/bin/konnect`
/// shape of its path, so its dead path is never recorded as the way back.
fn is_package_binary(command: &str) -> bool {
    let path = Path::new(command);
    if let Ok(canonical) = path.canonicalize() {
        if let Some(package) = canonical.parent().and_then(Path::parent) {
            if package.join(MANIFEST).is_file() {
                return true;
            }
        }
    }
    !path.exists()
        && path.file_name().is_some_and(|name| name == "konnect")
        && path
            .parent()
            .is_some_and(|bin| bin.file_name().is_some_and(|name| name == "bin"))
        && path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|name| name.to_string_lossy().contains("1-kopper"))
}

/// Steps 2 to 4 for the package at `prefix`. Returns what it did.
pub fn activate(host: &mut dyn Host, prefix: &Path) -> Result<String, String> {
    let exe = prefix.join("bin/konnect");
    let exe_text = exe.to_string_lossy().into_owned();
    let mut done = Vec::new();
    // Read again: a running client rewrites its file often.
    let clients = Clients::read(host)?;

    // 2. The way back: the first original of each client, never overwritten.
    let record_path = rollback_file(prefix);
    let mut record = read_record(&record_path)?;
    let mut added = false;
    if let Some(entry) = &clients.claude {
        let command = entry["command"].as_str().unwrap_or_default();
        if !is_package_binary(command) && !record.contains_key("claude") {
            record.insert("claude".into(), entry.clone());
            added = true;
        }
    }
    if let Some(command) = &clients.codex {
        if !is_package_binary(command) && !record.contains_key("codex_command") {
            record.insert("codex_command".into(), json!(command));
            added = true;
        }
    }
    if added {
        write_record(&record_path, &record)?;
        done.push(format!(
            "Recorded the earlier konnect entries in {}.",
            record_path.display()
        ));
    }

    // 3. Guidance from this build.
    let home = host.home().to_path_buf();
    host.run(&exe, &["init".into()], &home)?;
    done.push("Installed the skills, agents and hooks for Claude.".into());
    if clients.codex.is_some() {
        host.run(
            &exe,
            &["init".into(), "--client".into(), "codex".into()],
            &home,
        )?;
        done.push("Installed the skills for Codex.".into());
    }

    // 4. The entries.
    let mut entry = clients
        .claude
        .clone()
        .unwrap_or_else(|| json!({"type": "stdio", "args": [], "env": {}}));
    entry["command"] = json!(exe_text);
    if entry["env"].get("KONNECT_FASTROUTE").is_some() {
        done.push("Note: Claude's entry sets KONNECT_FASTROUTE, which comes before the package's own fastroute.".into());
    }
    switch_claude(host, clients.claude.as_ref(), &entry)?;
    done.push(format!("Claude's konnect entry runs {exe_text}."));
    if clients.codex.is_some() {
        set_codex_command(&codex_config(host), &exe_text).map_err(|e| {
            format!("{e}; Claude was switched already, and `cargo xtask kopper-install --rollback` puts both back")
        })?;
        done.push(format!("Codex's konnect entry runs {exe_text}."));
    } else {
        done.push("Codex has no konnect entry; it was left alone.".into());
    }
    Ok(done.join("\n"))
}

/// Replaces Claude's user `konnect` entry with `entry`; on a failed add, puts
/// `before` back.
fn switch_claude(host: &mut dyn Host, before: Option<&Value>, entry: &Value) -> Result<(), String> {
    let claude = PathBuf::from("claude");
    let home = host.home().to_path_buf();
    let add = |host: &mut dyn Host, entry: &Value| {
        host.run(
            &claude,
            &[
                "mcp".into(),
                "add-json".into(),
                SERVER.into(),
                entry.to_string(),
                "-s".into(),
                "user".into(),
            ],
            &home,
        )
    };
    if before.is_some() {
        host.run(
            &claude,
            &[
                "mcp".into(),
                "remove".into(),
                SERVER.into(),
                "-s".into(),
                "user".into(),
            ],
            &home,
        )?;
    }
    if let Err(error) = add(host, entry) {
        let restored = match before {
            Some(before) => match add(host, before) {
                Ok(()) => "the earlier entry was put back".to_string(),
                Err(back) => format!(
                    "putting the earlier entry back failed too ({back}); `cargo xtask kopper-install --rollback` restores it from the record"
                ),
            },
            None => "there was no earlier entry".to_string(),
        };
        return Err(format!("{error}; {restored}"));
    }
    Ok(())
}

/// The `command` line of `[mcp_servers.konnect]` in Codex's TOML `text`: its line
/// index and value. `Ok(None)` when no table names `konnect`. An error when one
/// does but its command is not one plain quoted string on its own line, so nothing
/// is rewritten on a guess and nothing is reported as absent that is there.
pub fn codex_command(text: &str) -> Result<Option<(usize, String)>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut inside = false;
    let mut seen = false;
    for (index, line) in text.lines().enumerate() {
        let trimmed = strip_comment(line.trim());
        if let Some(name) = table_header(trimmed) {
            inside = matches!(
                name.as_str(),
                "mcp_servers.konnect" | "mcp_servers.\"konnect\""
            );
            seen |= inside || name.starts_with("mcp_servers.konnect.");
            continue;
        }
        if trimmed.contains("konnect") && trimmed.contains('{') && !inside {
            seen = true; // an inline table, such as `konnect = { command = … }`
        }
        if !inside {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("command") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        return toml_basic_string(line.trim().split_once('=').map_or("", |(_, v)| v.trim()))
            .map(|command| Some((index, command)))
            .ok_or_else(|| {
                format!(
                    "its konnect command `{}` is not one plain quoted path alone on its line (no comment after it)",
                    value.trim()
                )
            });
    }
    if seen {
        return Err("it has a konnect table this install cannot read (an inline table, a subtable or no plain `command` line)".into());
    }
    Ok(None)
}

/// A TOML table header's name (`[a.b]`), or `None` for any other line, an array
/// line included.
fn table_header(line: &str) -> Option<String> {
    let name = line.strip_prefix('[')?.strip_suffix(']')?;
    (!name.starts_with('[')
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.\"".contains(c)))
    .then(|| name.to_string())
}

/// `line` without a trailing `# comment` outside a basic string.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (index, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '#' if !quoted => return line[..index].trim_end(),
            _ => {}
        }
    }
    line
}

/// A TOML basic string with only `\\` and `\"` escapes, the whole of `value`.
fn toml_basic_string(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                _ => return None,
            },
            '"' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

fn toml_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Rewrites the `command` of Codex's `konnect` entry to `command`, keeping every
/// other byte, the line ending and indentation, the file's mode, and any link to
/// it (the file it points at is the one rewritten). The file as it was before the
/// first switch is kept once as `config.toml.before-1-kopper`.
fn set_codex_command(path: &Path, command: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let target = path
        .canonicalize()
        .map_err(|e| format!("resolve {}: {e}", path.display()))?;
    let text =
        std::fs::read_to_string(&target).map_err(|e| format!("read {}: {e}", target.display()))?;
    let (index, _) = codex_command(&text)?
        .ok_or_else(|| format!("{} has no konnect command any more", target.display()))?;
    let bom = if text.starts_with('\u{feff}') {
        "\u{feff}"
    } else {
        ""
    };
    let body = &text[bom.len()..];
    let mut lines: Vec<&str> = body.split_inclusive('\n').collect();
    let line = lines[index];
    let indent = &line[..line.len() - line.trim_start().len()];
    let ending = if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let replaced = format!("{indent}command = {}{ending}", toml_quote(command));
    lines[index] = &replaced;
    let new_text = format!("{bom}{}", lines.concat());
    let backup = path.with_file_name("config.toml.before-1-kopper");
    if !kopper_package::present(&backup) {
        std::fs::copy(&target, &backup).map_err(|e| format!("keep {}: {e}", backup.display()))?;
    }
    let mode = std::fs::metadata(&target)
        .map_err(|e| format!("read {}: {e}", target.display()))?
        .permissions()
        .mode();
    let staged = target.with_file_name("config.toml.1-kopper-new");
    std::fs::write(&staged, new_text).map_err(|e| format!("write {}: {e}", staged.display()))?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("set the mode of {}: {e}", staged.display()))?;
    std::fs::rename(&staged, &target).map_err(|e| format!("replace {}: {e}", target.display()))
}

/// Whether `command` names a Konnect binary, so its `init` is Konnect's.
fn is_konnect(command: &str) -> bool {
    Path::new(command)
        .file_name()
        .is_some_and(|name| name == "konnect")
}

/// The way back for the package at `prefix`. Puts every recorded entry back first,
/// then runs each one's `init`; a failure is reported and does not stop the rest.
pub fn rollback(host: &mut dyn Host, prefix: &Path) -> Result<String, String> {
    let record_path = rollback_file(prefix);
    let record = read_record(&record_path)?;
    if record.is_empty() {
        return Err(format!(
            "no record of the earlier entries at {}",
            record_path.display()
        ));
    }
    let home = host.home().to_path_buf();
    let mut done = Vec::new();
    let mut failed = Vec::new();
    let mut inits: Vec<(String, Vec<String>)> = Vec::new();

    if let Some(entry) = record.get("claude") {
        let command = entry["command"].as_str().unwrap_or_default().to_string();
        let restored =
            read_claude(&home).and_then(|current| switch_claude(host, current.as_ref(), entry));
        match restored {
            Ok(()) => {
                done.push(format!("Claude's konnect entry runs {command} again."));
                inits.push((command, vec!["init".into()]));
            }
            Err(e) => failed.push(format!("Claude: {e}")),
        }
    }
    if let Some(command) = record.get("codex_command").and_then(Value::as_str) {
        let path = codex_config(host);
        let restored = read_codex(&path).and_then(|current| match current {
            Some(_) => set_codex_command(&path, command),
            None => Err(format!(
                "{} has no konnect entry to put back",
                path.display()
            )),
        });
        match restored {
            Ok(()) => {
                done.push(format!("Codex's konnect entry runs {command} again."));
                inits.push((
                    command.to_string(),
                    vec!["init".into(), "--client".into(), "codex".into()],
                ));
            }
            Err(e) => failed.push(format!("Codex: {e}")),
        }
    }
    for (command, args) in inits {
        if !is_konnect(&command) {
            done.push(format!(
                "{command} is not a konnect binary, so its init was not run."
            ));
            continue;
        }
        match host.run(Path::new(&command), &args, &home) {
            Ok(()) => done.push(format!("Ran {command} {}.", args.join(" "))),
            Err(e) => failed.push(e),
        }
    }
    if !failed.is_empty() {
        return Err(format!(
            "{}\nNot done:\n{}\nThe record is kept at {}; run the rollback again after fixing these.",
            done.join("\n"),
            failed.join("\n"),
            record_path.display()
        ));
    }
    let used = record_path.with_extension("json.used");
    std::fs::rename(&record_path, &used)
        .map_err(|e| format!("set the record aside as {}: {e}", used.display()))?;
    done.push(format!("The record is set aside as {}.", used.display()));
    Ok(done.join("\n"))
}

#[cfg(test)]
#[path = "kopper_install_tests.rs"]
mod tests;
