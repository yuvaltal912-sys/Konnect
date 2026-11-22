//! Each test packages two small files that stand in for the binaries; neither is
//! ever run.

use super::*;
use std::os::unix::fs::PermissionsExt;

struct Bench {
    home: tempfile::TempDir,
    konnect: PathBuf,
    fastroute: PathBuf,
    pin: Pin,
}

impl Bench {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let src = home.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let konnect = executable(&src, "konnect", "konnect stand-in");
        let fastroute = executable(&src, "fastroute", "fastroute stand-in");
        let pin = Pin {
            build: "x18".into(),
            engine_commit: "1e94751".into(),
            sha256: sha256_of(&fastroute).unwrap(),
        };
        Self {
            home,
            konnect,
            fastroute,
            pin,
        }
    }

    fn prefix(&self) -> PathBuf {
        self.home.path().join(DEFAULT_PREFIX)
    }

    fn request(&self, prefix: &Path) -> Request {
        Request {
            prefix: prefix.to_path_buf(),
            konnect: KonnectSource {
                binary: self.konnect.clone(),
                version: Some("0.13.0".into()),
                git_commit: Some("abc".into()),
                git_clean: Some(true),
            },
            fastroute: self.fastroute.clone(),
            pin: self.pin.clone(),
            homes: vec![self.home.path().to_path_buf()],
        }
    }
}

fn executable(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn the_repo_pin_parses() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../packaging/1-kopper/fastroute.json"),
    )
    .unwrap();
    let pin = Pin::parse(&text).unwrap();
    assert_eq!(pin.build, "x19");
    assert!(Pin::parse(&text.replace(&pin.sha256, "abc")).is_err());
    assert!(Pin::parse(&text.replace(&pin.sha256, &pin.sha256.to_uppercase())).is_err());
    assert!(Pin::parse("{}").is_err());
}

#[test]
fn a_package_holds_both_binaries_and_their_hashes() {
    let bench = Bench::new();
    let packaged = package(&bench.request(&bench.prefix())).unwrap();
    let prefix = bench.prefix().canonicalize().unwrap();
    assert_eq!(packaged.prefix, prefix);
    assert!(packaged.previous.is_none());
    for (name, source) in [("konnect", &bench.konnect), ("fastroute", &bench.fastroute)] {
        let copy = prefix.join("bin").join(name);
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            std::fs::read(source).unwrap()
        );
        assert_eq!(mode(&copy), 0o755, "{name}");
        assert_eq!(
            packaged.manifest[name]["sha256"],
            sha256_of(source).unwrap()
        );
    }
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(prefix.join(MANIFEST)).unwrap()).unwrap();
    assert_eq!(written, packaged.manifest);
    assert_eq!(written["fastroute"]["build"], "x18");
    assert_eq!(written["konnect"]["git_commit"], "abc");
    assert_eq!(written["name"], NAME);
    assert!(!present(&prefix.with_file_name("1-kopper.new")));
}

#[test]
fn a_fastroute_that_is_not_the_pinned_build_is_refused() {
    let bench = Bench::new();
    let mut request = bench.request(&bench.prefix());
    request.pin.sha256 = "0".repeat(64);
    let error = package(&request).unwrap_err();
    assert!(
        error.contains("not that of the pinned build x18"),
        "{error}"
    );
    assert!(!present(&bench.prefix()));
    assert!(!present(&bench.prefix().with_file_name("1-kopper.new")));
}

#[test]
fn the_installed_konnect_is_never_a_prefix() {
    let bench = Bench::new();
    let installed = bench.home.path().join(INSTALLED_KONNECT);
    std::fs::create_dir_all(installed.join("bin")).unwrap();
    let before = executable(&installed.join("bin"), "konnect", "installed");
    for prefix in [
        installed.clone(),
        installed.join("bin"),
        installed.join("1-kopper"),
        installed.parent().unwrap().to_path_buf(),
        bench.home.path().to_path_buf(),
    ] {
        let error = package(&bench.request(&prefix)).unwrap_err();
        assert!(error.contains("installed Konnect"), "{prefix:?}: {error}");
    }
    // Through a link to it, too.
    let link = bench.home.path().join("link");
    std::os::unix::fs::symlink(&installed, &link).unwrap();
    let error = package(&bench.request(&link.join("pkg"))).unwrap_err();
    assert!(error.contains("installed Konnect"), "{error}");
    assert_eq!(std::fs::read_to_string(before).unwrap(), "installed");
    assert_eq!(std::fs::read_dir(installed.join("bin")).unwrap().count(), 1);
}

#[test]
fn a_relative_prefix_or_one_with_dot_dot_is_refused() {
    let bench = Bench::new();
    let error = package(&bench.request(Path::new("1-kopper"))).unwrap_err();
    assert!(error.contains("not absolute"), "{error}");
    let sneaky = bench.prefix().join("../konnect");
    let error = package(&bench.request(&sneaky)).unwrap_err();
    assert!(error.contains("`..`"), "{error}");
}

#[test]
fn a_folder_that_is_not_a_package_is_never_replaced() {
    let bench = Bench::new();
    let prefix = bench.prefix();
    std::fs::create_dir_all(&prefix).unwrap();
    std::fs::write(prefix.join("mine.txt"), "keep").unwrap();
    let error = package(&bench.request(&prefix)).unwrap_err();
    assert!(error.contains("not a 1 Kopper package"), "{error}");
    assert_eq!(
        std::fs::read_to_string(prefix.join("mine.txt")).unwrap(),
        "keep"
    );
    assert!(!present(&prefix.join("bin")));
}

#[test]
fn packaging_again_keeps_the_earlier_package_as_the_way_back() {
    let bench = Bench::new();
    let prefix = bench.prefix();
    package(&bench.request(&prefix)).unwrap();
    std::fs::write(prefix.join("first.txt"), "first").unwrap();
    let packaged = package(&bench.request(&prefix)).unwrap();
    let previous = prefix.with_file_name("1-kopper.previous");
    assert_eq!(packaged.previous, Some(previous.canonicalize().unwrap()));
    assert_eq!(
        std::fs::read_to_string(previous.join("first.txt")).unwrap(),
        "first"
    );
    assert!(!present(&prefix.join("first.txt")));
    assert!(is_package(&prefix));

    // A third run replaces the older way back with the second package and leaves
    // no folder of the swap behind.
    std::fs::write(prefix.join("second.txt"), "second").unwrap();
    package(&bench.request(&prefix)).unwrap();
    assert!(present(&previous.join("second.txt")));
    assert!(!present(&previous.join("first.txt")));
    assert!(!present(&prefix.with_file_name("1-kopper.old")));
    assert!(!present(&prefix.with_file_name("1-kopper.new")));
}

#[test]
fn a_previous_folder_that_is_not_a_package_is_never_removed() {
    let bench = Bench::new();
    let prefix = bench.prefix();
    package(&bench.request(&prefix)).unwrap();
    let previous = prefix.with_file_name("1-kopper.previous");
    std::fs::create_dir_all(&previous).unwrap();
    std::fs::write(previous.join("mine.txt"), "keep").unwrap();
    let error = package(&bench.request(&prefix)).unwrap_err();
    assert!(error.contains("1-kopper.previous"), "{error}");
    assert_eq!(
        std::fs::read_to_string(previous.join("mine.txt")).unwrap(),
        "keep"
    );
    assert!(is_package(&prefix));
}

#[test]
fn a_staging_folder_left_by_an_interrupted_run_is_refused() {
    let bench = Bench::new();
    let staging = bench.prefix().with_file_name("1-kopper.new");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("half.txt"), "half").unwrap();
    let error = package(&bench.request(&bench.prefix())).unwrap_err();
    assert!(error.contains("interrupted run"), "{error}");
    assert_eq!(
        std::fs::read_to_string(staging.join("half.txt")).unwrap(),
        "half"
    );
    assert!(!present(&bench.prefix()));
}

#[test]
fn a_binary_that_is_not_an_executable_file_is_refused() {
    let bench = Bench::new();
    std::fs::set_permissions(&bench.konnect, std::fs::Permissions::from_mode(0o644)).unwrap();
    let error = package(&bench.request(&bench.prefix())).unwrap_err();
    assert!(error.contains("not executable"), "{error}");
    let mut request = bench.request(&bench.prefix());
    request.konnect.binary = bench.home.path().to_path_buf();
    let error = package(&request).unwrap_err();
    assert!(error.contains("not a regular file"), "{error}");
    assert!(!present(&bench.prefix()));
}

#[test]
fn the_workspace_version_is_read_from_the_repo() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let version = workspace_version(repo).unwrap();
    assert!(version.split('.').count() == 3, "{version}");
}

/// The guard holds under every home it is given: with `HOME` on a scratch folder,
/// the account's own installed Konnect is still refused.
#[test]
fn the_installed_konnect_of_every_home_is_guarded() {
    let bench = Bench::new();
    let account = tempfile::tempdir().unwrap();
    let installed = account.path().join(INSTALLED_KONNECT);
    std::fs::create_dir_all(&installed).unwrap();
    let mut request = bench.request(&installed.join("pkg"));
    request.homes.push(account.path().to_path_buf());
    let error = package(&request).unwrap_err();
    assert!(error.contains("installed Konnect"), "{error}");
    assert!(!present(&installed.join("pkg")));
    assert!(account_home().is_some_and(|home| home.is_absolute()));
}

/// Only a real folder whose manifest names 1 Kopper, with `bin/`, is a package:
/// any folder that merely holds a `1-kopper.json`, or a link to a package, is not.
#[test]
fn only_a_package_this_task_made_counts_as_one() {
    let bench = Bench::new();
    let prefix = bench.prefix();
    package(&bench.request(&prefix)).unwrap();
    assert!(is_package(&prefix));
    let other = bench.home.path().join("other");
    std::fs::create_dir_all(other.join("bin")).unwrap();
    std::fs::write(other.join(MANIFEST), r#"{"name": "something else"}"#).unwrap();
    assert!(!is_package(&other));
    std::fs::write(other.join(MANIFEST), "not json").unwrap();
    assert!(!is_package(&other));
    let link = bench.home.path().join("link");
    std::os::unix::fs::symlink(&prefix, &link).unwrap();
    assert!(!is_package(&link));
    let no_bin = bench.home.path().join("no-bin");
    std::fs::create_dir_all(&no_bin).unwrap();
    std::fs::copy(prefix.join(MANIFEST), no_bin.join(MANIFEST)).unwrap();
    assert!(!is_package(&no_bin));

    // A `.previous` that holds a foreign manifest is never removed.
    let previous = prefix.with_file_name("1-kopper.previous");
    std::fs::create_dir_all(previous.join("bin")).unwrap();
    std::fs::write(previous.join(MANIFEST), r#"{"name": "something else"}"#).unwrap();
    let error = package(&bench.request(&prefix)).unwrap_err();
    assert!(error.contains("1-kopper.previous"), "{error}");
    assert!(present(&previous.join(MANIFEST)));
}

#[test]
fn an_old_folder_left_by_an_interrupted_swap_is_refused() {
    let bench = Bench::new();
    let prefix = bench.prefix();
    package(&bench.request(&prefix)).unwrap();
    let old = prefix.with_file_name("1-kopper.old");
    std::fs::create_dir_all(&old).unwrap();
    let error = package(&bench.request(&prefix)).unwrap_err();
    assert!(error.contains("1-kopper.old"), "{error}");
    assert!(present(&old));
    assert!(!present(&prefix.with_file_name("1-kopper.previous")));
}
