//! What it takes to run Freerouting here, and how to get what is missing.
//!
//! A Freerouting JAR is built for one Java version and refuses to start on an
//! older one, with a class-loading error that says nothing about what to
//! install. So the JAR is asked what it needs (the class-file version of its
//! own main class, which no release note can get wrong), a Java that meets it
//! is looked for in the usual places, and when something is missing the answer
//! is the exact next step.
//!
//! **Why guidance and not a downloader.** Fetching and running a 60 MB
//! executable from the network is a decision for the person at the keyboard,
//! not a side effect of a routing call. Freerouting's licence (GPL-3.0) would
//! allow the download; the steps below are the same commands a person would run,
//! and `check_freerouting` verifies the result the way a routing call will use it.
//!
//! **Which version.** Any 2.x JAR starts, but releases differ in completion,
//! speed and whether two runs give the same result. [`RECOMMENDED`] names the
//! release `cargo xtask freerouting-bench` measured best, with the measured
//! reason, and [`version_policy`] compares the installed JAR against it.

use serde::Serialize;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where releases are published.
const RELEASES: &str = "https://github.com/freerouting/freerouting/releases";
/// Java's class-file major version is its release number plus this.
const CLASS_VERSION_OFFSET: u32 = 44;
/// For a JAR whose class files cannot be read: Freerouting 2.x needs Java 21.
const JAVA_FOR_FREEROUTING_2: u32 = 21;
const JAVA_FOR_FREEROUTING_1: u32 = 17;

/// The Freerouting release Konnect recommends, and the measurement behind it.
/// Update it only from a `cargo xtask freerouting-bench` run (see `DEV.md`),
/// quoting counts, never board names or results from a real project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecommendedRelease {
    pub version: &'static str,
    pub jar_name: &'static str,
    pub url: &'static str,
    /// SHA-256 of the release JAR, as GitHub's release API publishes it.
    pub sha256: &'static str,
    /// The Java release the JAR needs: its main class's class-file version
    /// minus 44 (2.5.0's is 69, so Java 25).
    pub java_major: u32,
    /// The measured reason, in one sentence.
    pub reason: &'static str,
    /// When, with what and over which corpus it was measured.
    pub measured: &'static str,
}

pub(crate) const RECOMMENDED: RecommendedRelease = RecommendedRelease {
    version: "2.5.0",
    jar_name: "freerouting-2.5.0.jar",
    url:
        "https://github.com/freerouting/freerouting/releases/download/v2.5.0/freerouting-2.5.0.jar",
    sha256: "f6f51bb02245e8e717f9359bd260cc9c5c0b1bc0acc8b7cb2cd5b8ffeb5de3c7",
    java_major: 25,
    reason: "on 10 boards of 2 to 114 parts exported by Konnect it routed all 10 completely with no clearance violation, against 7 for 2.1.0 and 5 each for 2.3.0 and 2.4.1; its two runs laid the same copper on all 10, against 2 of 7 for 2.1.0; and on the boards every release completed its wiring was as short, with as few vias, as the best of the others.",
    measured: "2026-11-08 with cargo xtask freerouting-bench: Freerouting 2.1.0, 2.3.0, 2.4.1 and 2.5.0 on Java 25.0.5, 2 runs per board, a 120 s job budget, over 5 KiCad 10 demo boards exported by Konnect with pours omitted and 5 test fixtures. A second corpus exported by KiCad's own exporter (7 demos with pours, and the fixtures) also ranked 2.5.0 first. On its completed boards 2.5.0 left 202 segments below their net class width (necked down, on 4 boards) and 1 acute corner, where 2.1.0 left none and 13.",
};

/// A Freerouting version such as `2.1.0` named in `--version` output or a log
/// line (`Freerouting v2.1.0 (build-date: …)`, `Freerouting 2.1.0`). A
/// pre-release suffix is kept (`2.5.0-SNAPSHOT`), so it is never taken for the
/// release.
pub(crate) fn version_in_output(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let at = line.find("Freerouting ")? + "Freerouting ".len();
        let rest = line[at..].strip_prefix('v').unwrap_or(&line[at..]);
        dotted_version(rest)
    })
}

/// `2.3.0` from a file name such as `freerouting-2.3.0.jar`.
pub(crate) fn version_in_file_name(jar: &Path) -> Option<String> {
    let name = jar.file_name()?.to_string_lossy();
    let stem = name.strip_suffix(".jar").unwrap_or(&name);
    dotted_version(stem.trim_start_matches(|c: char| !c.is_ascii_digit()))
}

/// Suffixes that mark a build as not the release (`-SNAPSHOT`, `-rc1`).
/// Others, such as `-executable`, are packaging and are dropped.
const PRE_RELEASE_MARKERS: &[&str] = &["snapshot", "alpha", "beta", "rc", "dev", "pre"];

/// The leading `N.N` or `N.N.N` of `text`, with a pre-release suffix if one
/// follows directly.
fn dotted_version(text: &str) -> Option<String> {
    let taken: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let version = taken.trim_end_matches('.');
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() < 2 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let mut version = version.to_string();
    if version.len() == taken.len() {
        if let Some(tag) = text[taken.len()..].strip_prefix('-') {
            let tag: String = tag
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
                .collect();
            let tag = tag.trim_end_matches('.');
            let lower = tag.to_ascii_lowercase();
            if PRE_RELEASE_MARKERS.iter().any(|m| lower.starts_with(m)) {
                version.push('-');
                version.push_str(tag);
            }
        }
    }
    Some(version)
}

/// The number list of a version, padded to three parts so `2.5` is `2.5.0`.
fn version_key(version: &str) -> Vec<u64> {
    let mut key: Vec<u64> = version
        .split('-')
        .next()
        .unwrap_or("")
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    while key.len() < 3 {
        key.push(0);
    }
    key
}

/// Order two versions by their numbers, zero-padded: `2.5` equals `2.5.0`.
pub(crate) fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (version_key(a), version_key(b));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a.cmp(&b)
}

/// The same release: equal numbers, zero-padded, and the same pre-release
/// suffix (none, for the release). `2.5` is `2.5.0`; `2.5.0-SNAPSHOT` is not.
pub(crate) fn same_version(a: &str, b: &str) -> bool {
    let suffix = |v: &str| v.split_once('-').map(|(_, tag)| tag.to_ascii_lowercase());
    compare_versions(a, b) == std::cmp::Ordering::Equal && suffix(a) == suffix(b)
}

/// The text of a version probe shows the JAR died while starting, after it
/// had printed its version: a Java exception trace, or a Freerouting ERROR
/// log line other than the expected complaint that a headless run was given
/// no board (2.1 to 2.5 print it after startup succeeded and exit 1).
///
/// Only text from the version line on is read: 2.5.0 logs a recoverable
/// look-and-feel error before its banner and carries on.
pub(crate) fn startup_crashed(text: &str) -> bool {
    const EXPECTED: &str = "Both an input file and an output file must be specified";
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let from = lines
        .iter()
        .position(|line| version_in_output(line).is_some())
        .unwrap_or(0);
    lines[from..].iter().any(|line| {
        if line.contains(EXPECTED) {
            return false;
        }
        let first = line
            .split(|c: char| c == ':' || c.is_whitespace())
            .next()
            .unwrap_or("");
        let exception_name =
            first.contains('.') && (first.ends_with("Exception") || first.ends_with("Error"));
        let frame = line.starts_with("at ") && line.ends_with(')') && line.contains('(');
        let error_level = line
            .split_whitespace()
            .take(3)
            .any(|word| matches!(word, "ERROR" | "SEVERE" | "FATAL"));
        line.starts_with("Exception in thread")
            || line.starts_with("Caused by:")
            || exception_name
            || frame
            || error_level
    })
}

/// SHA-256 of a file, as lowercase hex.
pub(crate) fn file_sha256(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// The installed JAR against [`RECOMMENDED`]. `version_output` is what the
/// JAR printed for `--version`, if it ran; the file name is the fallback.
/// `jar_sha256` is checked only when the versions match, so a JAR that says it
/// is the recommended release but is not the published file is named.
pub(crate) fn version_policy(
    jar: Option<&Path>,
    version_output: Option<&str>,
    jar_sha256: Option<&str>,
) -> Value {
    version_policy_for(&RECOMMENDED, jar, version_output, jar_sha256)
}

pub(crate) fn version_policy_for(
    recommended: &RecommendedRelease,
    jar: Option<&Path>,
    version_output: Option<&str>,
    jar_sha256: Option<&str>,
) -> Value {
    let (installed, basis) = match (version_output.and_then(version_in_output), jar) {
        (Some(version), _) => (Some(version), Some("the JAR's --version output")),
        (None, Some(jar)) => match version_in_file_name(jar) {
            Some(version) => (Some(version), Some("the JAR's file name")),
            None => (None, None),
        },
        (None, None) => (None, None),
    };
    let matches = installed
        .as_deref()
        .map(|v| same_version(v, recommended.version));
    let sha_matches = match (matches, jar_sha256) {
        (Some(true), Some(sha)) => Some(sha.eq_ignore_ascii_case(recommended.sha256)),
        _ => None,
    };
    let get = format!(
        "Download {} from {} (SHA-256 {}; it needs Java {} or newer) and pass it as jar_path, or put it where check_freerouting looks.",
        recommended.jar_name, recommended.url, recommended.sha256, recommended.java_major
    );
    let warning = match (jar, installed.as_deref(), matches, sha_matches) {
        (None, ..) => None,
        (Some(_), None, ..) => Some(format!(
            "The installed Freerouting's version could not be read, so it cannot be compared with the recommended {}. {get}",
            recommended.version
        )),
        (Some(_), Some(version), Some(false), _) => {
            let relation = match compare_versions(version, recommended.version) {
                std::cmp::Ordering::Less => "older than",
                std::cmp::Ordering::Greater => "newer than, and not measured against,",
                std::cmp::Ordering::Equal => "different from",
            };
            Some(format!(
                "The installed Freerouting {version} is {relation} the recommended {}: {} It still routes; {get}",
                recommended.version, recommended.reason
            ))
        }
        (Some(_), Some(version), Some(true), Some(false)) => Some(format!(
            "The installed JAR says it is Freerouting {version}, but its SHA-256 is not the published release's ({}). It may be a rebuild or a damaged download. {get}",
            recommended.sha256
        )),
        _ => None,
    };
    json!({
        "recommended_version": recommended.version,
        "recommended_jar": recommended.jar_name,
        "recommended_url": recommended.url,
        "recommended_sha256": recommended.sha256,
        "recommended_java_major": recommended.java_major,
        "reason": recommended.reason,
        "measured": recommended.measured,
        "installed_version": installed,
        "installed_version_basis": basis,
        "matches_recommended": matches,
        "sha256_matches_release": sha_matches,
        "warning": warning,
    })
}

/// Which Java a JAR needs, and how that was learned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct JarRequirement {
    pub required_java_major: u32,
    /// The JAR's own class-file major version, when it could be read.
    pub class_file_major: Option<u32>,
    pub basis: String,
}

/// A Java that starts, and which version it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct JavaFound {
    pub command: PathBuf,
    pub major: Option<u32>,
    pub version_output: String,
}

/// Which Java the JAR needs: read from its main class, else from its file name.
pub(crate) fn jar_requirement(jar: &Path) -> JarRequirement {
    match read_main_class_version(jar) {
        Some((class, major)) if major > CLASS_VERSION_OFFSET => JarRequirement {
            required_java_major: major - CLASS_VERSION_OFFSET,
            class_file_major: Some(major),
            basis: format!("class file version {major} of {class}, read from the JAR"),
        },
        _ => {
            let name = jar
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let release = name
                .trim_start_matches(|c: char| !c.is_ascii_digit())
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|n| n.parse::<u32>().ok());
            let required = match release {
                Some(release) if release >= 2 => JAVA_FOR_FREEROUTING_2,
                _ => JAVA_FOR_FREEROUTING_1,
            };
            JarRequirement {
                required_java_major: required,
                class_file_major: None,
                basis: format!(
                    "inferred from the file name {name:?}: the JAR's class files could not be read"
                ),
            }
        }
    }
}

/// `(main class, class-file major version)` of a JAR.
fn read_main_class_version(jar: &Path) -> Option<(String, u32)> {
    let file = std::fs::File::open(jar).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut manifest = String::new();
    archive
        .by_name("META-INF/MANIFEST.MF")
        .ok()?
        .take(64 * 1024)
        .read_to_string(&mut manifest)
        .ok()?;
    let class = manifest
        .lines()
        .find_map(|line| line.strip_prefix("Main-Class:"))?
        .trim()
        .to_string();
    let mut header = [0u8; 8];
    archive
        .by_name(&format!("{}.class", class.replace('.', "/")))
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    if header[..4] != [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }
    Some((class, u32::from(u16::from_be_bytes([header[6], header[7]]))))
}

/// The Java release number in `java -version` output (`"11.0.31"`, `"1.8.0_292"`,
/// `"21"`, `"22-ea"`).
pub(crate) fn parse_java_major(output: &str) -> Option<u32> {
    let quoted = output.split('"').nth(1)?;
    let mut parts = quoted.split(|c: char| !c.is_ascii_digit());
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

/// Every Java that starts, in the order a person would reach for them: the one
/// named, `JAVA_HOME`, the one on `PATH`, then the usual install places.
pub(crate) async fn discover_java(explicit: Option<&Path>) -> Vec<JavaFound> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = explicit {
        candidates.push(path.to_path_buf());
    }
    // A test names its own Java and must not meet the machine's.
    #[cfg(test)]
    if ONLY_EXPLICIT.with(std::cell::Cell::get) {
        return probe_all(candidates).await;
    }
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        candidates.push(Path::new(&home).join("bin").join(java_binary()));
    }
    candidates.push(PathBuf::from("java"));
    candidates.extend(installed_javas());
    probe_all(candidates).await
}

#[cfg(test)]
thread_local! {
    pub(crate) static ONLY_EXPLICIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

async fn probe_all(candidates: Vec<PathBuf>) -> Vec<JavaFound> {
    let mut found: Vec<JavaFound> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for command in candidates {
        // The same install is often reachable by several paths.
        let identity = command.canonicalize().unwrap_or_else(|_| command.clone());
        if command != Path::new("java") && seen.contains(&identity) {
            continue;
        }
        if let Some(java) = probe_java(&command).await {
            // `java` on PATH and the JDK it points at are one install.
            if found
                .iter()
                .any(|other| other.version_output == java.version_output)
            {
                continue;
            }
            seen.push(identity);
            found.push(java);
        }
    }
    found
}

fn java_binary() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// `bin/java` of each JDK in the places installers put them.
fn installed_javas() -> Vec<PathBuf> {
    let mut roots: Vec<(PathBuf, &str)> = Vec::new();
    if cfg!(target_os = "macos") {
        roots.push((
            "/Library/Java/JavaVirtualMachines".into(),
            "Contents/Home/bin",
        ));
        if let Some(home) = dirs::home_dir() {
            roots.push((
                home.join("Library/Java/JavaVirtualMachines"),
                "Contents/Home/bin",
            ));
        }
        roots.push(("/opt/homebrew/opt".into(), "bin"));
        roots.push(("/usr/local/opt".into(), "bin"));
    } else if cfg!(windows) {
        for base in [
            "C:\\Program Files\\Java",
            "C:\\Program Files\\Eclipse Adoptium",
            "C:\\Program Files\\Microsoft",
        ] {
            roots.push((base.into(), "bin"));
        }
    } else {
        roots.push(("/usr/lib/jvm".into(), "bin"));
    }
    let mut out = Vec::new();
    for (root, tail) in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        names.sort();
        for dir in names {
            let is_jdk = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .is_some_and(|n| {
                    n.contains("jdk")
                        || n.contains("jre")
                        || n.contains("java")
                        || n.contains("corretto")
                        || n.contains("temurin")
                });
            if is_jdk {
                out.push(dir.join(tail).join(java_binary()));
            }
        }
    }
    out
}

async fn probe_java(command: &Path) -> Option<JavaFound> {
    let mut process = tokio::process::Command::new(command);
    process.arg("-version").kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(10), process.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let version_output = text.trim().to_string();
    Some(JavaFound {
        major: parse_java_major(&version_output),
        command: command.to_path_buf(),
        version_output,
    })
}

/// The first Java that meets `required`.
pub(crate) fn pick(found: &[JavaFound], required: u32) -> Option<&JavaFound> {
    found
        .iter()
        .find(|java| java.major.is_some_and(|m| m >= required))
}

/// What to do next, as exact steps.
pub(crate) fn guidance(
    jar: Option<&Path>,
    requirement: Option<&JarRequirement>,
    found: &[JavaFound],
) -> Value {
    let required = requirement.map_or(JAVA_FOR_FREEROUTING_2, |r| r.required_java_major);
    let java_ok = pick(found, required).is_some();
    let status = match (jar.is_some(), java_ok) {
        (true, true) => "ready",
        (true, false) => "needs_java",
        (false, true) => "needs_jar",
        (false, false) => "needs_jar_and_java",
    };
    let mut steps: Vec<String> = Vec::new();
    if jar.is_none() {
        steps.push(format!(
            "Download {} ({}, SHA-256 {}, needs Java {} or newer), the release Konnect recommends: {} Every release is at {RELEASES}.",
            RECOMMENDED.jar_name, RECOMMENDED.url, RECOMMENDED.sha256, RECOMMENDED.java_major, RECOMMENDED.reason
        ));
        steps.push(
            "Put it in a KiCad plugin folder (KiCad's Plugin and Content Manager installs one there), or anywhere and pass jar_path to check_freerouting and route_specctra_dsn.".to_string(),
        );
    }
    if !java_ok {
        let seen = if found.is_empty() {
            "no Java was found".to_string()
        } else {
            format!("found Java {}", {
                let mut majors: Vec<u32> = found.iter().filter_map(|j| j.major).collect();
                majors.sort_unstable();
                majors.dedup();
                majors
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
        };
        steps.push(format!(
            "Install Java {required} or newer ({seen}). macOS: `brew install openjdk@{required}`, then `sudo ln -sfn $(brew --prefix openjdk@{required})/libexec/openjdk.jdk /Library/Java/JavaVirtualMachines/openjdk-{required}.jdk`. Debian/Ubuntu: `sudo apt install openjdk-{required}-jre`. Windows: install the Eclipse Temurin {required} JRE."
        ));
        steps.push(
            "Run check_freerouting again. Konnect looks in JAVA_HOME, on PATH and in the usual JDK folders, so a second Java beside an older one is found; pass java_path to name one."
                .to_string(),
        );
    }
    json!({
        "status": status,
        "required_java_major": required,
        "required_java_basis": requirement.map(|r| r.basis.clone()),
        "java_found": found
            .iter()
            .map(|j| json!({ "command": j.command, "major": j.major }))
            .collect::<Vec<_>>(),
        "steps": steps,
        "jar_source": RELEASES,
        "jar_licence": "GPL-3.0",
        "version_policy": format!(
            "Freerouting {} is recommended: {} Any 2.x release still starts: Konnect reads the Java version a JAR needs from the JAR itself.",
            RECOMMENDED.version, RECOMMENDED.reason
        ),
        "why_no_download": "Konnect does not download or install anything for you: fetching and running a 60 MB program from the network is your decision, and the steps above are the same commands. check_freerouting verifies the result the way a routing call will use it.",
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A JAR with a manifest and a main class whose header says `class_major`.
    pub(crate) fn fake_jar(dir: &Path, name: &str, class_major: u16) -> PathBuf {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("META-INF/MANIFEST.MF", options).unwrap();
        zip.write_all(b"Manifest-Version: 1.0\r\nMain-Class: app.freerouting.Freerouting\r\n\r\n")
            .unwrap();
        zip.start_file("app/freerouting/Freerouting.class", options)
            .unwrap();
        let mut header = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0];
        header.extend(class_major.to_be_bytes());
        header.extend([0u8; 16]);
        zip.write_all(&header).unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn java_versions_are_read_from_every_style_of_output() {
        for (output, major) in [
            ("openjdk version \"11.0.31\" 2026-04-21 LTS", Some(11)),
            ("java version \"1.8.0_292\"", Some(8)),
            ("openjdk version \"21\" 2023-09-19", Some(21)),
            ("openjdk version \"21.0.4\" 2024-07-16", Some(21)),
            ("openjdk version \"22-ea\" 2024-03-19", Some(22)),
            ("command not found", None),
            ("", None),
        ] {
            assert_eq!(parse_java_major(output), major, "{output}");
        }
    }

    #[test]
    fn a_jar_says_which_java_it_needs() {
        let dir = tempfile::tempdir().unwrap();
        for (class_major, java) in [(65u16, 21u32), (61, 17), (55, 11)] {
            let jar = fake_jar(dir.path(), "freerouting-2.1.0.jar", class_major);
            let requirement = jar_requirement(&jar);
            assert_eq!(requirement.required_java_major, java);
            assert_eq!(requirement.class_file_major, Some(u32::from(class_major)));
            assert!(requirement.basis.contains("read from the JAR"));
        }
    }

    #[test]
    fn a_jar_that_cannot_be_read_falls_back_to_its_file_name_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("freerouting-2.3.0.jar");
        std::fs::write(&jar, b"not a zip").unwrap();
        let requirement = jar_requirement(&jar);
        assert_eq!(requirement.required_java_major, 21);
        assert_eq!(requirement.class_file_major, None);
        assert!(requirement.basis.contains("inferred from the file name"));
        let old = dir.path().join("freerouting-1.9.0.jar");
        std::fs::write(&old, b"x").unwrap();
        assert_eq!(jar_requirement(&old).required_java_major, 17);
    }

    fn java(major: u32) -> JavaFound {
        JavaFound {
            command: PathBuf::from(format!("/jvm/{major}/bin/java")),
            major: Some(major),
            version_output: format!("openjdk version \"{major}\""),
        }
    }

    #[test]
    fn the_first_java_that_is_new_enough_is_chosen() {
        let found = [java(11), java(21), java(17)];
        assert_eq!(pick(&found, 21).unwrap().major, Some(21));
        assert_eq!(pick(&found, 17).unwrap().major, Some(21));
        assert!(pick(&found, 22).is_none());
        assert!(pick(&[java(11)], 21).is_none());
    }

    #[test]
    fn guidance_names_the_missing_piece_and_the_exact_step() {
        let jar = Path::new("/x/freerouting-2.1.0.jar");
        let need = JarRequirement {
            required_java_major: 21,
            class_file_major: Some(65),
            basis: "class file version 65".to_string(),
        };
        let old = guidance(Some(jar), Some(&need), &[java(11)]);
        assert_eq!(old["status"], "needs_java");
        let steps = old["steps"].to_string();
        assert!(steps.contains("brew install openjdk@21"), "{steps}");
        assert!(steps.contains("found Java 11"), "{steps}");
        assert!(!steps.contains("Download"), "the JAR is there: {steps}");
        assert_eq!(
            guidance(Some(jar), Some(&need), &[java(21)])["status"],
            "ready"
        );
        assert_eq!(guidance(None, None, &[java(21)])["status"], "needs_jar");
        let none = guidance(None, None, &[]);
        assert_eq!(none["status"], "needs_jar_and_java");
        assert!(none["steps"].to_string().contains("releases"));
        assert!(none["steps"].to_string().contains("no Java was found"));
        assert_eq!(none["jar_licence"], "GPL-3.0");
        assert!(none["why_no_download"]
            .as_str()
            .unwrap()
            .contains("your decision"));
    }

    const MEASURED: RecommendedRelease = RecommendedRelease {
        version: "2.5.0",
        jar_name: "freerouting-2.5.0.jar",
        url: "https://example.invalid/freerouting-2.5.0.jar",
        sha256: "ab12",
        java_major: 25,
        reason: "it routed every board, the same way twice.",
        measured: "2026-11-08 over 9 boards",
    };

    #[test]
    fn versions_are_read_from_output_and_file_names() {
        for (output, version) in [
            (
                "2026-11-08 INFO   Freerouting v2.1.0 (build-date: 2025-04-12)",
                Some("2.1.0"),
            ),
            ("Freerouting 2.1.0", Some("2.1.0")),
            (
                "x\nINFO Freerouting v2.5.0-SNAPSHOT (build",
                Some("2.5.0-SNAPSHOT"),
            ),
            ("Freerouting v2.6.0-rc1", Some("2.6.0-rc1")),
            ("Freerouting started", None),
            ("Error: could not open the jar", None),
        ] {
            assert_eq!(version_in_output(output).as_deref(), version, "{output}");
        }
        for (name, version) in [
            ("freerouting-2.3.0.jar", Some("2.3.0")),
            ("freerouting-2.4.1-executable.jar", Some("2.4.1")),
            ("freerouting-2.5.0-SNAPSHOT.jar", Some("2.5.0-SNAPSHOT")),
            ("freerouting.jar", None),
            ("freerouting-2.jar", None),
        ] {
            assert_eq!(
                version_in_file_name(Path::new(name)).as_deref(),
                version,
                "{name}"
            );
        }
    }

    #[test]
    fn the_recommended_release_is_reported_with_its_measured_reason() {
        let jar = Path::new("/x/freerouting-2.5.0.jar");
        let policy = version_policy_for(&MEASURED, Some(jar), None, Some("AB12"));
        assert_eq!(policy["recommended_version"], "2.5.0");
        assert_eq!(policy["reason"], MEASURED.reason);
        assert_eq!(policy["measured"], MEASURED.measured);
        assert_eq!(policy["recommended_java_major"], 25);
        assert_eq!(policy["installed_version"], "2.5.0");
        assert_eq!(policy["installed_version_basis"], "the JAR's file name");
        assert_eq!(policy["matches_recommended"], true);
        assert_eq!(policy["sha256_matches_release"], true);
        assert_eq!(policy["warning"], Value::Null);
    }

    #[test]
    fn a_different_installed_version_is_warned_about_with_the_reason_and_the_fix() {
        let jar = Path::new("/x/freerouting-2.5.0.jar");
        // The JAR's own output wins over its file name.
        let policy = version_policy_for(
            &MEASURED,
            Some(jar),
            Some("INFO   Freerouting v2.1.0 (build-date: 2025-04-12)"),
            None,
        );
        assert_eq!(policy["installed_version"], "2.1.0");
        assert_eq!(
            policy["installed_version_basis"],
            "the JAR's --version output"
        );
        assert_eq!(policy["matches_recommended"], false);
        assert_eq!(policy["sha256_matches_release"], Value::Null);
        let warning = policy["warning"].as_str().unwrap();
        assert!(
            warning.contains("2.1.0 is older than the recommended 2.5.0"),
            "{warning}"
        );
        assert!(warning.contains(MEASURED.reason), "{warning}");
        assert!(
            warning.contains(MEASURED.url) && warning.contains("ab12"),
            "{warning}"
        );
        assert!(warning.contains("needs Java 25 or newer"), "{warning}");

        let newer = version_policy_for(
            &MEASURED,
            Some(Path::new("/x/freerouting-2.10.0.jar")),
            None,
            None,
        );
        let warning = newer["warning"].as_str().unwrap();
        assert!(
            warning.contains("2.10.0 is newer than, and not measured against,"),
            "{warning}"
        );
    }

    #[test]
    fn versions_compare_by_zero_padded_numbers() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("2.5", "2.5.0"), Equal);
        assert_eq!(compare_versions("2.5.0", "2.5"), Equal);
        assert_eq!(compare_versions("2.10", "2.5.0"), Greater);
        assert_eq!(compare_versions("2.1", "2.5.0"), Less);
        assert!(same_version("2.5", "2.5.0"));
        assert!(same_version("2.5.0", "2.5.0"));
        assert!(!same_version("2.5.1", "2.5.0"));
        // The release has no suffix; a snapshot is not it.
        assert!(!same_version("2.5.0-SNAPSHOT", "2.5.0"));
        assert!(same_version("2.5.0-snapshot", "2.5.0-SNAPSHOT"));
    }

    #[test]
    fn a_short_version_is_the_same_release_and_its_digest_is_checked() {
        let policy = version_policy_for(
            &MEASURED,
            Some(Path::new("/x/freerouting-2.5.jar")),
            None,
            Some("AB12"),
        );
        assert_eq!(policy["installed_version"], "2.5");
        assert_eq!(policy["matches_recommended"], true, "{policy}");
        assert_eq!(policy["sha256_matches_release"], true, "{policy}");
        assert_eq!(policy["warning"], Value::Null, "{policy}");
    }

    #[test]
    fn a_snapshot_is_reported_as_one_and_never_as_the_release() {
        let policy = version_policy_for(
            &MEASURED,
            None,
            Some("INFO Freerouting v2.5.0-SNAPSHOT (build-date: 2026-10-05)"),
            Some("AB12"),
        );
        assert_eq!(policy["installed_version"], "2.5.0-SNAPSHOT");
        assert_eq!(policy["matches_recommended"], false, "{policy}");
        assert_eq!(policy["sha256_matches_release"], Value::Null);
        // No JAR was given, so there is nothing to warn about; with one, it
        // says the snapshot differs from the release.
        let policy = version_policy_for(
            &MEASURED,
            Some(Path::new("/x/freerouting.jar")),
            Some("INFO Freerouting v2.5.0-SNAPSHOT"),
            None,
        );
        let warning = policy["warning"].as_str().unwrap();
        assert!(
            warning.contains("2.5.0-SNAPSHOT is different from the recommended 2.5.0"),
            "{warning}"
        );
    }

    /// What the 2.5.0 release printed for this exact probe (headless, no
    /// board, isolated home), recorded on macOS on 2026-11-09: two WARN lines
    /// and the expected complaint, twice. A healthy start.
    #[test]
    fn the_real_2_5_0_probe_output_is_a_healthy_start() {
        let text = "2026-11-09 12:45:48.575 INFO   Freerouting v2.5.0 (build-date: 2026-10-05)
2026-11-09 12:45:48.834 INFO   Hardware: 15 CPU cores, 565 CPU score, 6144 MB RAM
2026-11-09 12:45:48.835 WARN   Unknown command line argument: --version
2026-11-09 12:45:48.840 WARN   GUI is disabled and you don't have a console available, so the only feedback from Freerouting is in the log.
2026-11-09 12:45:49.963 ERROR  Both an input file and an output file must be specified with command line arguments if you are running in CLI mode.
2026-11-09 12:45:49.963 ERROR  Both an input file and an output file must be specified with command line arguments if you are running in CLI mode.
";
        assert_eq!(version_in_output(text).as_deref(), Some("2.5.0"));
        assert!(!startup_crashed(text));
    }

    #[test]
    fn a_banner_followed_by_a_crash_is_a_startup_crash_but_a_missing_board_is_not() {
        let banner = "2026-11-09 INFO   Freerouting v2.5.0 (build-date: 2026-10-05)";
        // What a healthy headless run with no board prints (stdout, then stderr).
        let healthy = format!(
            "{banner}\n2026-11-09 ERROR  Both an input file and an output file must be specified with command line arguments if you are running in CLI mode."
        );
        assert!(!startup_crashed(&healthy));
        assert!(!startup_crashed(banner));
        for crash in [
            "Exception in thread \"main\" java.lang.NullPointerException",
            "java.lang.NullPointerException: Cannot invoke \"String.length()\"",
            "\tat app.freerouting.startup.GlobalSettingsBootstrap.initialize(GlobalSettingsBootstrap.java:1)",
            "Caused by: java.io.IOException: disk full",
            "2026-11-09 ERROR  Could not read freerouting.json",
        ] {
            let text = format!("{banner}\n{crash}");
            assert!(startup_crashed(&text), "{crash}");
            // The expected complaint beside a real crash does not hide it.
            assert!(startup_crashed(&format!("{healthy}\n{crash}")), "{crash}");
        }
        // 2.5.0 logs a recoverable look-and-feel error before its banner.
        let before = format!("2026-11-09 ERROR  Look and feel unavailable\n{healthy}");
        assert!(!startup_crashed(&before));
    }

    #[test]
    fn the_recommended_version_with_another_digest_is_named() {
        let jar = Path::new("/x/freerouting-2.5.0.jar");
        let policy = version_policy_for(&MEASURED, Some(jar), None, Some("ffff"));
        assert_eq!(policy["matches_recommended"], true);
        assert_eq!(policy["sha256_matches_release"], false);
        assert!(policy["warning"]
            .as_str()
            .unwrap()
            .contains("not the published release"));
    }

    #[test]
    fn an_unreadable_version_or_no_jar_is_said_plainly() {
        let unknown = version_policy_for(
            &MEASURED,
            Some(Path::new("/x/freerouting.jar")),
            Some("boom"),
            None,
        );
        assert_eq!(unknown["installed_version"], Value::Null);
        assert_eq!(unknown["matches_recommended"], Value::Null);
        assert!(unknown["warning"]
            .as_str()
            .unwrap()
            .contains("could not be read"));
        let none = version_policy_for(&MEASURED, None, None, None);
        assert_eq!(none["recommended_version"], "2.5.0");
        assert_eq!(none["warning"], Value::Null);
        assert_eq!(none["installed_version"], Value::Null);
    }

    #[test]
    fn the_shipped_recommendation_is_complete_and_names_a_published_jar() {
        let r = RECOMMENDED;
        assert_eq!(r.jar_name, format!("freerouting-{}.jar", r.version));
        assert!(
            r.url.starts_with(RELEASES) && r.url.ends_with(r.jar_name),
            "{}",
            r.url
        );
        assert_eq!(r.sha256.len(), 64);
        assert!(r.java_major >= 21, "Freerouting 2.x needs Java 21 or newer");
        assert!(r
            .sha256
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(
            !r.reason.contains("PENDING") && r.reason.ends_with('.'),
            "{}",
            r.reason
        );
        assert!(r.measured.contains("freerouting-bench"), "{}", r.measured);
        let steps = guidance(None, None, &[])["steps"].to_string();
        assert!(steps.contains(r.url), "{steps}");
    }

    /// Against the real recommended JAR: its digest is the pinned one and its
    /// main class asks for the pinned Java.
    #[test]
    #[ignore = "set FREEROUTING_JAR to the recommended release JAR"]
    fn the_pinned_digest_and_java_match_the_real_release() {
        let jar = PathBuf::from(std::env::var_os("FREEROUTING_JAR").expect("set FREEROUTING_JAR"));
        assert_eq!(file_sha256(&jar).unwrap(), RECOMMENDED.sha256);
        let requirement = jar_requirement(&jar);
        assert_eq!(
            requirement.required_java_major, RECOMMENDED.java_major,
            "{requirement:?}"
        );
        assert!(
            requirement.basis.contains("read from the JAR"),
            "{requirement:?}"
        );
        let policy = version_policy(Some(&jar), None, Some(RECOMMENDED.sha256));
        assert_eq!(policy["warning"], Value::Null, "{policy}");
    }

    #[test]
    fn a_file_digest_is_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            file_sha256(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_named_java_is_probed_and_one_that_does_not_start_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let good = crate::tools::cli::test_support::write_script(
            dir.path(),
            "java21",
            "#!/bin/sh\necho 'openjdk version \"21.0.4\" 2024-07-16' >&2\nexit 0\n",
            "",
        );
        let found = discover_java(Some(&good)).await;
        let first = &found[0];
        assert_eq!(
            (first.command.as_path(), first.major),
            (good.as_path(), Some(21))
        );
        let broken = crate::tools::cli::test_support::write_script(
            dir.path(),
            "broken",
            "#!/bin/sh\nexit 3\n",
            "",
        );
        let found = discover_java(Some(&broken)).await;
        assert!(found.iter().all(|j| j.command != broken));
    }
}
