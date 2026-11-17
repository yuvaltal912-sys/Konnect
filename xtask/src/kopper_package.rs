//! `cargo xtask kopper-package`: puts 1 Kopper, Konnect plus its pinned fastroute
//! build, into a folder of its own:
//!
//! ```text
//! <prefix>/bin/konnect      the Konnect release build
//! <prefix>/bin/fastroute    the fastroute build `packaging/1-kopper/fastroute.json` pins
//! <prefix>/1-kopper.json    what was packaged: versions and SHA-256 of both
//! ```
//!
//! Konnect finds `bin/fastroute` beside itself when the manifest is one folder up
//! (`fastroute::bundled_path`), so the package needs no configuration. Simulation
//! stays off unless a configuration turns it on (`simulation_enabled`).
//!
//! What it refuses, before it writes anything:
//! - a fastroute file whose SHA-256 is not the pinned one;
//! - a prefix that is, holds or lies inside the installed Konnect
//!   (`~/.local/share/konnect`, under `HOME` and under the account's own home), a
//!   relative prefix, or one with `..`;
//! - an existing prefix, or `<prefix>.previous`, that is not an earlier package (a
//!   real folder, not a link, whose `1-kopper.json` names 1 Kopper, with `bin/`);
//! - a `<prefix>.new` or `<prefix>.old` left by an interrupted run.
//!
//! It copies files and never runs either binary (fastroute runs only sandboxed,
//! under Konnect). This task changes no MCP client; `kopper-install` does that on
//! top of it. A package already at the prefix becomes `<prefix>.previous`.

use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

/// The manifest's file name in the package (`fastroute::PACKAGE_MANIFEST`).
pub const MANIFEST: &str = "1-kopper.json";

/// The manifest's `name`.
pub const NAME: &str = "1 Kopper";

/// Where the installed Konnect lives, under a home folder. Never written.
const INSTALLED_KONNECT: &str = ".local/share/konnect";

/// The default prefix, under `HOME`.
pub const DEFAULT_PREFIX: &str = ".local/share/1-kopper";

/// Where fastroute is by default, under `HOME` (Konnect's own default place).
const DEFAULT_FASTROUTE: &str = ".local/share/fastroute/candidate/fastroute";

/// The pinned fastroute build, as `packaging/1-kopper/fastroute.json` records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub build: String,
    pub engine_commit: String,
    pub sha256: String,
}

impl Pin {
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| format!("the pin is not JSON: {error}"))?;
        let field = |name: &str| {
            value[name]
                .as_str()
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("the pin has no `{name}` text"))
        };
        let pin = Self {
            build: field("build")?,
            engine_commit: field("engine_commit")?,
            sha256: field("sha256")?,
        };
        if pin.sha256.len() != 64
            || !pin
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(format!(
                "the pin's sha256 `{}` is not 64 lowercase hex digits",
                pin.sha256
            ));
        }
        Ok(pin)
    }
}

/// Where the Konnect binary comes from, and what the manifest says of its source.
#[derive(Debug, Clone)]
pub struct KonnectSource {
    pub binary: PathBuf,
    /// The workspace version and commit it was built from, when this run built it.
    pub version: Option<String>,
    pub git_commit: Option<String>,
    pub git_clean: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub prefix: PathBuf,
    pub konnect: KonnectSource,
    pub fastroute: PathBuf,
    pub pin: Pin,
    /// Home folders the installed Konnect is guarded under: `HOME` and the
    /// account's own home, which differ when `HOME` points at a scratch folder.
    pub homes: Vec<PathBuf>,
}

/// What was packaged.
#[derive(Debug)]
pub struct Packaged {
    pub prefix: PathBuf,
    pub manifest: serde_json::Value,
    /// The earlier package, kept as the way back.
    pub previous: Option<PathBuf>,
}

pub const USAGE: &str = "[--prefix <dir>] [--konnect <file>] [--fastroute <file>]";

/// The package arguments `args` give (see [`USAGE`]), with the defaults under `HOME`.
/// Without `--konnect` the release is built here and that build is packaged.
pub fn request_from_args(args: &[String], repo: &Path) -> Result<Request, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    let mut prefix = home.join(DEFAULT_PREFIX);
    let mut konnect = None;
    let mut fastroute = home.join(DEFAULT_FASTROUTE);
    let mut index = 0;
    while index < args.len() {
        let value = || {
            args.get(index + 1)
                .map(PathBuf::from)
                .ok_or_else(|| format!("{} needs a path", args[index]))
        };
        match args[index].as_str() {
            "--prefix" => prefix = value()?,
            "--konnect" => konnect = Some(value()?),
            "--fastroute" => fastroute = value()?,
            other => return Err(format!("unknown argument {other}; arguments: {USAGE}")),
        }
        index += 2;
    }
    let pin_path = repo.join("packaging/1-kopper/fastroute.json");
    let pin = Pin::parse(
        &std::fs::read_to_string(&pin_path)
            .map_err(|error| format!("read {}: {error}", pin_path.display()))?,
    )?;
    let mut homes = vec![home];
    homes.extend(account_home());
    // Checked here too so a bad prefix is refused before a release build.
    checked_prefix(&prefix, &homes)?;
    let konnect = match konnect {
        Some(binary) => KonnectSource {
            binary,
            version: None,
            git_commit: None,
            git_clean: None,
        },
        None => build_release(repo)?,
    };
    Ok(Request {
        prefix,
        konnect,
        fastroute,
        pin,
        homes,
    })
}

/// `cargo xtask kopper-package` ([`USAGE`]).
pub fn run(args: &[String], repo: &Path) -> Result<(), String> {
    let packaged = package(&request_from_args(args, repo)?)?;
    report(&packaged);
    println!(
        "No MCP client was changed: `cargo xtask kopper-install` also switches them, or point the konnect entry at {} yourself.",
        packaged.prefix.join("bin/konnect").display()
    );
    Ok(())
}

/// Prints what was packaged.
pub fn report(packaged: &Packaged) {
    println!("1 Kopper packaged at {}:", packaged.prefix.display());
    println!(
        "{}",
        serde_json::to_string_pretty(&packaged.manifest).expect("a JSON value prints")
    );
    if let Some(previous) = &packaged.previous {
        println!("The earlier package is kept at {}.", previous.display());
    }
}

/// The account's home folder from the user database, whatever `HOME` says.
pub(crate) fn account_home() -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is to a live local of the size given; on success
    // `entry`'s strings point into `buffer`, which outlives their use below.
    let code = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if code != 0 || result.is_null() || entry.pw_dir.is_null() {
        return None;
    }
    // SAFETY: `pw_dir` is a NUL-terminated string inside `buffer`.
    let dir = unsafe { CStr::from_ptr(entry.pw_dir) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes())))
}

/// Builds the release here and returns the executable cargo reports.
fn build_release(repo: &Path) -> Result<KonnectSource, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = std::process::Command::new(cargo)
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "konnect",
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(repo)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|error| format!("start cargo build: {error}"))?;
    if !output.status.success() {
        return Err(format!("cargo build --release failed: {}", output.status));
    }
    let binary = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| {
            message["reason"] == "compiler-artifact" && message["target"]["name"] == "konnect"
        })
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .ok_or("cargo build reported no konnect executable")?;
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    Ok(KonnectSource {
        binary,
        version: workspace_version(repo),
        git_commit: git(&["rev-parse", "HEAD"]),
        git_clean: git(&["status", "--porcelain", "--untracked-files=no"])
            .map(|out| out.is_empty()),
    })
}

/// `[workspace.package] version` of the repo's `Cargo.toml`.
fn workspace_version(repo: &Path) -> Option<String> {
    let text = std::fs::read_to_string(repo.join("Cargo.toml")).ok()?;
    let section = text.split("[workspace.package]").nth(1)?;
    section
        .lines()
        .take_while(|line| !line.trim_start().starts_with('['))
        .find_map(|line| {
            let rest = line
                .trim()
                .strip_prefix("version")?
                .trim_start()
                .strip_prefix('=')?;
            Some(rest.trim().trim_matches('"').to_string())
        })
}

/// `<prefix>` with `suffix` added to its folder name.
fn sibling(prefix: &Path, suffix: &str) -> PathBuf {
    let name = prefix
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    prefix.with_file_name(format!("{name}{suffix}"))
}

/// Makes the package. Every refusal comes before the first write.
pub fn package(request: &Request) -> Result<Packaged, String> {
    let prefix = checked_prefix(&request.prefix, &request.homes)?;
    let konnect_sha = executable_sha256(&request.konnect.binary, "the Konnect binary")?;
    let fastroute_sha = executable_sha256(&request.fastroute, "the fastroute binary")?;
    if fastroute_sha != request.pin.sha256 {
        return Err(format!(
            "the fastroute binary {} has SHA-256 {fastroute_sha}, not that of the pinned build {} ({}); nothing was written",
            request.fastroute.display(),
            request.pin.build,
            request.pin.sha256
        ));
    }
    let staging = sibling(&prefix, ".new");
    let previous = sibling(&prefix, ".previous");
    let old = sibling(&prefix, ".old");
    for leftover in [&staging, &old] {
        if present(leftover) {
            return Err(format!(
                "{} is left from an interrupted run; look at it and remove it first",
                leftover.display()
            ));
        }
    }
    let replacing = present(&prefix);
    for (folder, applies) in [(&prefix, replacing), (&previous, replacing)] {
        if applies && present(folder) && !is_package(folder) {
            return Err(format!(
                "{} exists and is not a 1 Kopper package (a folder whose {MANIFEST} names {NAME}, with bin/); nothing was written",
                folder.display()
            ));
        }
    }

    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    // Exclusive: a folder that appeared since the check above is never taken over.
    std::fs::create_dir(&staging).map_err(|e| format!("create {}: {e}", staging.display()))?;
    let written = (|| {
        let bin = staging.join("bin");
        std::fs::create_dir(&bin).map_err(|e| format!("create {}: {e}", bin.display()))?;
        copy_verified(&request.konnect.binary, &bin.join("konnect"), &konnect_sha)?;
        copy_verified(&request.fastroute, &bin.join("fastroute"), &fastroute_sha)?;
        let manifest = serde_json::json!({
            "name": NAME,
            "konnect": {
                "sha256": konnect_sha,
                "version": request.konnect.version,
                "git_commit": request.konnect.git_commit,
                "git_clean": request.konnect.git_clean,
            },
            "fastroute": {
                "build": request.pin.build,
                "engine_commit": request.pin.engine_commit,
                "sha256": fastroute_sha,
            },
            "packaged_unix_seconds": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or_default(),
        });
        let text = serde_json::to_string_pretty(&manifest).expect("a JSON value prints") + "\n";
        let path = staging.join(MANIFEST);
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok::<_, String>(manifest)
    })();
    let manifest = match written {
        Ok(manifest) => manifest,
        Err(error) => {
            // Only the staging folder this run created above.
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };

    if !replacing {
        std::fs::rename(&staging, &prefix).map_err(|e| {
            format!(
                "move {} to {}: {e}; the new package is at {}",
                staging.display(),
                prefix.display(),
                staging.display()
            )
        })?;
        return Ok(Packaged {
            prefix,
            manifest,
            previous: None,
        });
    }
    swap(&prefix, &staging, &previous, &old)?;
    Ok(Packaged {
        prefix,
        manifest,
        previous: Some(previous),
    })
}

/// Puts `staging` at `prefix` and the package that was there at `previous`. The
/// older `previous` (a package, checked) is set aside as `old` and removed only
/// once the swap is done; each failure undoes the steps before it and says where
/// every folder is.
fn swap(prefix: &Path, staging: &Path, previous: &Path, old: &Path) -> Result<(), String> {
    let had_previous = present(previous);
    if had_previous {
        std::fs::rename(previous, old).map_err(|e| {
            format!(
                "move {} to {}: {e}; nothing else was moved, and the new package is at {}",
                previous.display(),
                old.display(),
                staging.display()
            )
        })?;
    }
    let undo_old = || {
        if had_previous {
            match std::fs::rename(old, previous) {
                Ok(()) => String::new(),
                Err(e) => format!(
                    " The older package could not be put back at {} ({e}); it is at {}.",
                    previous.display(),
                    old.display()
                ),
            }
        } else {
            String::new()
        }
    };
    if let Err(e) = std::fs::rename(prefix, previous) {
        return Err(format!(
            "move {} to {}: {e}; the package in use is unchanged and the new one is at {}.{}",
            prefix.display(),
            previous.display(),
            staging.display(),
            undo_old()
        ));
    }
    if let Err(e) = std::fs::rename(staging, prefix) {
        let back = match std::fs::rename(previous, prefix) {
            Ok(()) => format!("the package in use is back at {}", prefix.display()),
            Err(back) => format!(
                "the package in use could not be put back ({back}); it is at {}",
                previous.display()
            ),
        };
        return Err(format!(
            "move {} to {}: {e}; {back}, and the new one is at {}.{}",
            staging.display(),
            prefix.display(),
            staging.display(),
            undo_old()
        ));
    }
    if had_previous {
        // The swap is done; an older package left behind only blocks the next run,
        // which names it.
        if let Err(e) = std::fs::remove_dir_all(old) {
            eprintln!(
                "The package is in place, but the older one at {} could not be removed ({e}); remove it before the next run.",
                old.display()
            );
        }
    }
    Ok(())
}

/// The prefix, absolute and free of `..`, with its existing part resolved; refused
/// when it is, holds or lies inside the installed Konnect under any of `homes`.
pub(crate) fn checked_prefix(prefix: &Path, homes: &[PathBuf]) -> Result<PathBuf, String> {
    if !prefix.is_absolute() {
        return Err(format!("the prefix {} is not absolute", prefix.display()));
    }
    if prefix.components().any(|part| part == Component::ParentDir) {
        return Err(format!("the prefix {} has a `..`", prefix.display()));
    }
    let resolved = resolve_existing(prefix);
    for home in homes {
        let installed = resolve_existing(&home.join(INSTALLED_KONNECT));
        if resolved.starts_with(&installed) || installed.starts_with(&resolved) {
            return Err(format!(
                "the prefix {} is, holds or lies inside the installed Konnect {}, which is never written",
                prefix.display(),
                installed.display()
            ));
        }
    }
    Ok(resolved)
}

/// `path` with its longest existing leading part canonicalized (links followed)
/// and the rest appended.
fn resolve_existing(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    loop {
        if let Ok(canonical) = existing.canonicalize() {
            return rest
                .iter()
                .rev()
                .fold(canonical, |path, part| path.join(part));
        }
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Whether anything (even a broken link) is at `path`.
pub(crate) fn present(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Whether `path` is a package this task made: a real folder (not a link) whose
/// manifest names 1 Kopper, with a `bin` folder.
pub(crate) fn is_package(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
        && path.join("bin").is_dir()
        && std::fs::read_to_string(path.join(MANIFEST))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|manifest| manifest["name"] == NAME)
}

/// The SHA-256 of a regular, executable file.
fn executable_sha256(path: &Path, what: &str) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| format!("{what} {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{what} {} is not a regular file", path.display()));
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(format!("{what} {} is not executable", path.display()));
    }
    sha256_of(path)
}

pub(crate) fn sha256_of(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Copies `from` to `to` with mode 0755 and checks the copy has `sha256`.
fn copy_verified(from: &Path, to: &Path, sha256: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::copy(from, to)
        .map_err(|e| format!("copy {} to {}: {e}", from.display(), to.display()))?;
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("set the mode of {}: {e}", to.display()))?;
    let copied = sha256_of(to)?;
    if copied != sha256 {
        return Err(format!(
            "the copy {} has SHA-256 {copied}, not {sha256}",
            to.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "kopper_package_tests.rs"]
mod tests;
