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
            "Download the latest Freerouting 2.x release JAR (freerouting-<version>.jar) from {RELEASES}."
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
        "version_policy": "Any Freerouting 2.x release. Konnect reads the Java version a JAR needs from the JAR itself, so a newer release needs no change here. Konnect's own checks cover the 2.1 and 2.3 releases.",
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
