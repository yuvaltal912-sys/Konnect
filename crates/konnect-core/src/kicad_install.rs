//! KiCad installation discovery.
//!
//! Keep executable and bundled-library discovery behind this module so every
//! caller sees the same installations in the same order.  Discovery is
//! intentionally read-only: Windows and KiCad already persist install state,
//! while an explicit Konnect configuration remains the durable override.

use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
const SUPPORTED_VERSIONS: [&str; 3] = ["10.0", "9.0", "8.0"];

/// Find `kicad-cli`, honoring an explicit configured value first.
pub fn find_cli(configured: &str) -> Option<PathBuf> {
    resolve_configured(configured)
        .or_else(|| cli_candidates().into_iter().find(|path| path.is_file()))
}

/// Find the KiCad project-manager executable.
///
/// An explicit GUI path wins. If the configured CLI is an absolute path, its
/// sibling GUI is next so a selected KiCad version cannot be mixed with a
/// different installation's libraries or UI.
pub fn find_gui(configured_binary: &str, configured_cli: &str) -> Option<PathBuf> {
    resolve_explicit_path(configured_binary)
        .or_else(|| {
            resolve_configured(configured_cli)
                .and_then(|cli| installation_root_from_binary(&cli))
                .map(|root| root.join("bin").join(gui_filename()))
                .filter(|path| path.is_file())
        })
        .or_else(|| resolve_configured(configured_binary))
        .or_else(|| gui_candidates().into_iter().find(|path| path.is_file()))
}

/// Where libngspice was found, and everywhere that was checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NgspiceSearch {
    pub library: Option<PathBuf>,
    pub searched: Vec<PathBuf>,
}

/// Find libngspice.
///
/// A nonblank setting is the only place looked, so a typo is reported rather
/// than quietly replaced by another copy. A blank one looks beside the resolved
/// `kicad-cli` first, so a selected KiCad cannot run a different KiCad's engine,
/// then at the usual install locations.
pub fn find_ngspice_library(configured: &str, configured_cli: &str) -> NgspiceSearch {
    let configured = configured.trim();
    let searched = if configured.is_empty() {
        let mut paths = find_cli(configured_cli)
            .map(|cli| ngspice_near_cli(&cli))
            .unwrap_or_default();
        for path in ngspice_candidates() {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        paths
    } else {
        vec![PathBuf::from(configured)]
    };
    NgspiceSearch {
        library: searched.iter().find(|path| path.is_file()).cloned(),
        searched,
    }
}

/// Find the code-model folder for a library. As in KiCad, `spice2poly.cm` marks
/// it. Debian and Ubuntu put these in the `ngspice` package, not `libngspice0`,
/// so `None` is a normal answer there.
pub fn find_code_models_dir(configured: &str, library: &Path) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        return Some(PathBuf::from(configured)).filter(|path| path.is_dir());
    }
    let library_dir = library.parent()?;
    [
        Some(library_dir.join("ngspice")),
        library_dir
            .parent()
            .map(|root| root.join("lib").join("ngspice")),
    ]
    .into_iter()
    .flatten()
    .find(|dir| dir.join("spice2poly.cm").is_file())
}

fn ngspice_library_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["ngspice.dll", "libngspice-0.dll", "libngspice-1.dll"]
    } else if cfg!(target_os = "macos") {
        &["libngspice.0.dylib", "libngspice.dylib"]
    } else {
        &["libngspice.so.0", "libngspice.so"]
    }
}

/// `installation_root_from_binary` needs a `bin` folder, which a macOS bundle
/// does not have: `Contents/MacOS/kicad-cli` pairs with `Contents/PlugIns/sim`.
fn ngspice_near_cli(cli: &Path) -> Vec<PathBuf> {
    let Some(cli_dir) = cli.parent() else {
        return Vec::new();
    };
    let dirs = if cli_dir.file_name().is_some_and(|name| name == "MacOS") {
        cli_dir
            .parent()
            .map(|contents| vec![contents.join("PlugIns").join("sim")])
            .unwrap_or_default()
    } else {
        let mut dirs = vec![cli_dir.to_path_buf()];
        if let Some(root) = installation_root_from_binary(cli) {
            dirs.push(root.join("lib"));
        }
        dirs
    };
    dirs.iter()
        .flat_map(|dir| ngspice_library_names().iter().map(|name| dir.join(name)))
        .collect()
}

fn ngspice_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    let dirs: Vec<PathBuf> = installation_roots()
        .into_iter()
        .map(|root| root.join("bin"))
        .collect();
    #[cfg(target_os = "macos")]
    let dirs: Vec<PathBuf> = {
        let mut dirs = vec![PathBuf::from(
            "/Applications/KiCad/KiCad.app/Contents/PlugIns/sim",
        )];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(
                PathBuf::from(home).join("Applications/KiCad/KiCad.app/Contents/PlugIns/sim"),
            );
        }
        dirs.push(PathBuf::from("/opt/homebrew/lib"));
        dirs.push(PathBuf::from("/usr/local/lib"));
        dirs
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let dirs: Vec<PathBuf> = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/usr/local/lib",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();

    dirs.iter()
        .flat_map(|dir| ngspice_library_names().iter().map(|name| dir.join(name)))
        .collect()
}

fn cli_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        installation_roots()
            .into_iter()
            .map(|root| root.join("bin").join(cli_filename()))
            .collect()
    }
    #[cfg(target_os = "macos")]
    {
        let mut paths = vec![
            PathBuf::from("/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli"),
            PathBuf::from("/usr/local/bin/kicad-cli"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(
                PathBuf::from(home).join("Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli"),
            );
        }
        paths
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        vec![
            PathBuf::from("/usr/bin/kicad-cli"),
            PathBuf::from("/usr/local/bin/kicad-cli"),
        ]
    }
}

fn gui_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        installation_roots()
            .into_iter()
            .map(|root| root.join("bin").join(gui_filename()))
            .collect()
    }
    #[cfg(target_os = "macos")]
    {
        vec![PathBuf::from(
            "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad",
        )]
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        ["kicad"]
            .into_iter()
            .filter_map(|name| find_on_path(Path::new(name)))
            .collect()
    }
}

/// Existing roots that directly contain KiCad's `symbols`, `footprints`, and
/// `3dmodels` directories, newest supported KiCad first.
pub fn share_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(target_os = "windows")]
    for install in installation_roots() {
        push_existing_unique(&mut roots, install.join("share").join("kicad"));
    }

    #[cfg(target_os = "macos")]
    {
        push_existing_unique(
            &mut roots,
            PathBuf::from("/Applications/KiCad/KiCad.app/Contents/SharedSupport"),
        );
        push_existing_unique(&mut roots, PathBuf::from("/usr/local/share/kicad"));
        push_existing_unique(&mut roots, PathBuf::from("/opt/homebrew/share/kicad"));
        if let Some(home) = std::env::var_os("HOME") {
            push_existing_unique(
                &mut roots,
                PathBuf::from(home).join("Applications/KiCad/KiCad.app/Contents/SharedSupport"),
            );
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        for root in [
            PathBuf::from("/usr/share/kicad"),
            PathBuf::from("/usr/local/share/kicad"),
            PathBuf::from("/opt/kicad/share/kicad"),
            PathBuf::from("/var/lib/flatpak/app/org.kicad.KiCad/current/active/files/share/kicad"),
            PathBuf::from("/snap/kicad/current/usr/share/kicad"),
        ] {
            push_existing_unique(&mut roots, root);
        }
        if let Some(home) = std::env::var_os("HOME") {
            push_existing_unique(
                &mut roots,
                PathBuf::from(home).join(
                    ".local/share/flatpak/app/org.kicad.KiCad/current/active/files/share/kicad",
                ),
            );
        }
    }

    if roots.is_empty() {
        static WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        WARNED.get_or_init(|| {
            tracing::warn!(
                target: "konnect::kicad_install",
                "no KiCad bundled-library root was resolved; stock symbols, footprints, and 3D models may be unavailable; configure kicad_cli or the KICAD<major>_*_DIR variables"
            );
        });
    }

    roots
}

#[cfg(target_os = "windows")]
fn installation_roots() -> Vec<PathBuf> {
    static ROOTS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    ROOTS.get_or_init(discover_installation_roots).clone()
}

#[cfg(target_os = "windows")]
fn discover_installation_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for root in windows_registry_install_roots() {
        push_existing_unique(&mut roots, root);
    }
    for root in windows_well_known_install_roots(
        std::env::var_os("LOCALAPPDATA").as_deref(),
        std::env::var_os("ProgramFiles").as_deref(),
        std::env::var_os("ProgramFiles(x86)").as_deref(),
    ) {
        push_existing_unique(&mut roots, root);
    }
    roots
}

fn resolve_configured(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if configured.is_empty() {
        return None;
    }
    let path = PathBuf::from(configured);
    if path.is_file() {
        return Some(path);
    }
    if path.components().count() == 1 {
        return find_on_path(&path);
    }
    None
}

fn resolve_explicit_path(configured: &str) -> Option<PathBuf> {
    let path = PathBuf::from(configured.trim());
    (path.components().count() > 1 && path.is_file()).then_some(path)
}

fn find_on_path(filename: &Path) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(filename))
        .find(|path| path.is_file())
}

fn installation_root_from_binary(binary: &Path) -> Option<PathBuf> {
    let bin_dir = binary.parent()?;
    if !bin_dir
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("bin"))
    {
        return None;
    }
    bin_dir.parent().map(Path::to_path_buf)
}

fn push_existing_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path.is_dir() && !paths.contains(&path) {
        paths.push(path);
    }
}

#[cfg(any(target_os = "windows", test))]
fn cli_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "kicad-cli.exe"
    } else {
        "kicad-cli"
    }
}

fn gui_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "kicad.exe"
    } else {
        "kicad"
    }
}

#[cfg(target_os = "windows")]
fn windows_well_known_install_roots(
    local_app_data: Option<&std::ffi::OsStr>,
    program_files: Option<&std::ffi::OsStr>,
    program_files_x86: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(path) = local_app_data {
        bases.push(PathBuf::from(path).join("Programs").join("KiCad"));
    }
    if let Some(path) = program_files {
        bases.push(PathBuf::from(path).join("KiCad"));
    }
    if let Some(path) = program_files_x86 {
        bases.push(PathBuf::from(path).join("KiCad"));
    }
    for fallback in [
        r"C:\Program Files\KiCad",
        r"C:\KiCad",
        r"D:\KiCad",
        r"D:\Program Files\KiCad",
    ] {
        let fallback = PathBuf::from(fallback);
        if !bases.contains(&fallback) {
            bases.push(fallback);
        }
    }

    let mut roots = Vec::new();
    for version in SUPPORTED_VERSIONS {
        for base in &bases {
            roots.push(base.join(version));
        }
    }
    for base in bases {
        roots.push(base);
    }
    roots
}

#[cfg(target_os = "windows")]
fn windows_registry_install_roots() -> Vec<PathBuf> {
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };
    use winreg::RegKey;

    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let mut roots = Vec::new();

    for version in SUPPORTED_VERSIONS {
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
                let root = RegKey::predef(hive);
                let Ok(uninstall) = root.open_subkey_with_flags(UNINSTALL, KEY_READ | view) else {
                    continue;
                };
                for name in uninstall.enum_keys().flatten() {
                    let Ok(key) = uninstall.open_subkey_with_flags(&name, KEY_READ | view) else {
                        continue;
                    };
                    let display_name: String = key.get_value("DisplayName").unwrap_or_default();
                    let display_version: String =
                        key.get_value("DisplayVersion").unwrap_or_default();
                    if !display_name.to_ascii_lowercase().starts_with("kicad ")
                        || !(display_version.starts_with(version) || display_name.contains(version))
                    {
                        continue;
                    }
                    let location: String = key.get_value("InstallLocation").unwrap_or_default();
                    if !location.trim().is_empty() {
                        let path = PathBuf::from(location.trim());
                        if path.is_dir() && !roots.contains(&path) {
                            roots.push(path);
                        }
                    }
                }
            }
        }
    }

    // Older KiCad installers used vendor keys instead of an uninstall entry.
    for version in SUPPORTED_VERSIONS {
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
                let root = RegKey::predef(hive);
                for key_path in [
                    format!(r"SOFTWARE\KiCad\{version}"),
                    r"SOFTWARE\KiCad\KiCad".into(),
                ] {
                    let Ok(key) = root.open_subkey_with_flags(&key_path, KEY_READ | view) else {
                        continue;
                    };
                    let location: String = key
                        .get_value("InstallDir")
                        .or_else(|_| key.get_value(""))
                        .unwrap_or_default();
                    if !location.trim().is_empty() {
                        let path = PathBuf::from(location.trim());
                        if path.is_dir() && !roots.contains(&path) {
                            roots.push(path);
                        }
                    }
                }
            }
        }
    }

    roots
}

#[cfg(test)]
mod ngspice_tests {
    use super::*;

    #[test]
    fn a_configured_library_is_the_only_place_looked() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("typo.so");
        let search = find_ngspice_library(missing.to_str().unwrap(), "");
        assert_eq!(search.library, None);
        assert_eq!(search.searched, vec![missing]);
    }

    #[test]
    fn a_configured_library_that_exists_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("lib.so");
        std::fs::write(&library, "").unwrap();
        let search = find_ngspice_library(library.to_str().unwrap(), "");
        assert_eq!(search.library, Some(library));
    }

    #[test]
    fn a_macos_bundle_pairs_its_cli_with_its_plugins_folder() {
        let contents = Path::new("/Apps/KiCad.app/Contents");
        let paths = ngspice_near_cli(&contents.join("MacOS/kicad-cli"));
        assert!(!paths.is_empty());
        assert!(paths
            .iter()
            .all(|path| path.parent() == Some(&*contents.join("PlugIns").join("sim"))));
    }

    #[test]
    fn a_bin_folder_pairs_with_itself_and_its_lib_folder() {
        let paths = ngspice_near_cli(Path::new("/opt/kicad/bin/kicad-cli"));
        let parents: Vec<_> = paths.iter().filter_map(|path| path.parent()).collect();
        assert!(parents.contains(&Path::new("/opt/kicad/bin")));
        assert!(parents.contains(&Path::new("/opt/kicad/lib")));
    }

    #[test]
    fn code_models_sit_beside_the_library_or_in_the_lib_folder() {
        let root = tempfile::tempdir().unwrap();
        let beside = root.path().join("sim");
        std::fs::create_dir_all(beside.join("ngspice")).unwrap();
        std::fs::write(beside.join("ngspice/spice2poly.cm"), "").unwrap();
        assert_eq!(
            find_code_models_dir("", &beside.join("libngspice.dylib")),
            Some(beside.join("ngspice"))
        );

        let windows = root.path().join("win");
        std::fs::create_dir_all(windows.join("lib/ngspice")).unwrap();
        std::fs::write(windows.join("lib/ngspice/spice2poly.cm"), "").unwrap();
        assert_eq!(
            find_code_models_dir("", &windows.join("bin/ngspice.dll")),
            Some(windows.join("lib").join("ngspice"))
        );
    }

    #[test]
    fn a_folder_without_the_marker_is_not_a_code_model_folder() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("ngspice")).unwrap();
        assert_eq!(find_code_models_dir("", &root.path().join("lib.so")), None);
    }

    #[test]
    fn a_configured_code_model_folder_must_exist() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        assert_eq!(
            find_code_models_dir(missing.to_str().unwrap(), Path::new("/x/lib.so")),
            None
        );
        assert_eq!(
            find_code_models_dir(root.path().to_str().unwrap(), Path::new("/x/lib.so")),
            Some(root.path().to_path_buf())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn derives_install_root_from_cli() {
        let cli = Path::new(r"C:\Users\casey\AppData\Local\Programs\KiCad\10.0\bin\kicad-cli.exe");
        assert_eq!(
            installation_root_from_binary(cli),
            Some(PathBuf::from(
                r"C:\Users\casey\AppData\Local\Programs\KiCad\10.0"
            ))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn per_user_roots_are_newest_first() {
        use std::ffi::OsStr;

        let roots = windows_well_known_install_roots(
            Some(OsStr::new(r"C:\Users\casey\AppData\Local")),
            Some(OsStr::new(r"C:\Program Files")),
            None,
        );
        assert_eq!(
            roots[0],
            PathBuf::from(r"C:\Users\casey\AppData\Local\Programs\KiCad\10.0")
        );
        assert!(roots.contains(&PathBuf::from(
            r"C:\Users\casey\AppData\Local\Programs\KiCad\9.0"
        )));
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires a complete KiCad installation"]
    fn live_install_exposes_all_bundled_library_kinds() {
        let cli = find_cli("").expect("KiCad installation was not discovered");
        let install = installation_root_from_binary(&cli).expect("CLI was not under <root>/bin");
        let share = install.join("share").join("kicad");

        for kind in ["symbols", "footprints", "3dmodels"] {
            assert!(share.join(kind).is_dir(), "missing KiCad {kind} directory");
        }
        assert!(share_roots().contains(&share));
    }

    #[test]
    fn explicit_cli_and_sibling_gui_win() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let cli = bin.join(cli_filename());
        let gui = bin.join(gui_filename());
        std::fs::write(&cli, b"").unwrap();
        std::fs::write(&gui, b"").unwrap();

        assert_eq!(find_cli(cli.to_str().unwrap()), Some(cli.clone()));
        assert_eq!(find_gui(gui_filename(), cli.to_str().unwrap()), Some(gui));
    }
}
