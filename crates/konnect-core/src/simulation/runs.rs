//! Where runs live: `<state root>/simulation/<project hash>/<run id>/`.
//!
//! Outside the project, because raw files can reach hundreds of MB and projects
//! may sit in synced folders. The id starts with the project hash, so finding a
//! run later needs no scan.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const KEEP_RUNS: usize = 20;
const HASH_CHARS: usize = 12;
const STAMP_CHARS: usize = 15;
const SUFFIX_CHARS: usize = 4;
const CREATE_ATTEMPTS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunFolder {
    pub id: String,
    pub dir: PathBuf,
    /// Old runs that could not be removed. They cost disk, not this run.
    pub old_runs_kept: usize,
}

fn project_hash(schematic: &Path) -> std::io::Result<String> {
    let canonical = schematic.canonicalize()?;
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let mut hash = format!("{digest:x}");
    hash.truncate(HASH_CHARS);
    Ok(hash)
}

/// `yyyymmddThhmmss` in UTC. Days to a date is Howard Hinnant's `civil_from_days`.
fn stamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let clock = seconds % 86_400;
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}",
        clock / 3_600,
        clock / 60 % 60,
        clock % 60
    )
}

/// The schema enforces this shape too, and handlers check it again, because a
/// run id becomes a folder name.
pub(crate) fn is_valid_id(id: &str) -> bool {
    let hex = |text: &[u8]| {
        text.iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    };
    let digits = |text: &[u8]| text.iter().all(u8::is_ascii_digit);
    let bytes = id.as_bytes();
    let stamp_end = HASH_CHARS + 1 + STAMP_CHARS;
    bytes.len() == stamp_end + 1 + SUFFIX_CHARS
        && hex(&bytes[..HASH_CHARS])
        && bytes[HASH_CHARS] == b'-'
        && digits(&bytes[HASH_CHARS + 1..HASH_CHARS + 9])
        && bytes[HASH_CHARS + 9] == b'T'
        && digits(&bytes[HASH_CHARS + 10..stamp_end])
        && bytes[stamp_end] == b'-'
        && hex(&bytes[stamp_end + 1..])
}

/// A new, empty folder for this run, after dropping the project's oldest.
pub(crate) fn create(state_root: &Path, schematic: &Path) -> std::io::Result<RunFolder> {
    let hash = project_hash(schematic)?;
    let project = state_root.join("simulation").join(&hash);
    std::fs::create_dir_all(&project)?;
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let mut attempt = 0;
    loop {
        let suffix: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(SUFFIX_CHARS)
            .collect();
        let id = format!("{hash}-{}-{suffix}", stamp(seconds));
        let dir = project.join(&id);
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                let old_runs_kept = prune(&project, &id);
                return Ok(RunFolder {
                    id,
                    dir,
                    old_runs_kept,
                });
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    && attempt < CREATE_ATTEMPTS =>
            {
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Keep the newest runs, counting the current one, and return how many old ones
/// would not go. A failure here must never refuse a good run. Anything that is not
/// a real run folder stays, so a stray file or link is never followed or deleted.
fn prune(project: &Path, current: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(project) else {
        return 0;
    };
    let mut runs: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_valid_id(name) && name != current)
        .collect();
    runs.sort();
    let surplus = runs.len().saturating_sub(KEEP_RUNS - 1);
    let gone = |name: &&String| match std::fs::remove_dir_all(project.join(name)) {
        Ok(()) => false,
        // Another server pruned the same run first.
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    };
    runs[..surplus].iter().filter(gone).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schematic(dir: &Path) -> PathBuf {
        let path = dir.join("board.kicad_sch");
        std::fs::write(&path, "").unwrap();
        path
    }

    #[test]
    fn stamps_are_utc_dates() {
        assert_eq!(stamp(0), "19700101T000000");
        assert_eq!(stamp(951_782_400), "20000229T000000");
        assert_eq!(stamp(1_793_450_096), "20261031T123456");
        assert_eq!(stamp(1_735_689_599), "20241231T235959");
    }

    #[test]
    fn a_new_run_has_a_valid_id_under_its_project() {
        let folder = tempfile::tempdir().unwrap();
        let run = create(folder.path(), &schematic(folder.path())).unwrap();
        assert!(is_valid_id(&run.id), "{}", run.id);
        assert!(run.dir.is_dir());
        let project = run.dir.parent().unwrap();
        assert!(run
            .id
            .starts_with(&project.file_name().unwrap().to_string_lossy().to_string()));
        assert_eq!(project.parent().unwrap().file_name().unwrap(), "simulation");
    }

    #[test]
    fn only_the_exact_id_shape_is_valid() {
        assert!(is_valid_id("3fa9c1d27b04-20261030T133512-7f3a"));
        for bad in [
            "",
            "../../etc",
            "3fa9c1d27b04-20261030T133512",
            "3fa9c1d27b04-20261030T133512-7f3a-0",
            "3fa9c1d27b0-20261030T133512-7f3a",
            "3fa9c1d27b04-20261030T13351-7f3a",
            "3fa9c1d27b04-20261030X133512-7f3a",
            "3FA9C1D27B04-20261030T133512-7f3a",
            "3fa9c1d27b04-20261030T133512-7f3g",
            "3fa9c1d27b04-2026103aT133512-7f3a",
            "3fa9c1d27b04-20261030T13351a-7f3a",
            "3fa9c1d27b04-20261030T133512-7f3",
            "3fa9c1d27b04-2026103\u{5e9}T1234-0000",
            "\u{5e9}\u{5e9}\u{5e9}\u{5e9}\u{5e9}\u{5e9}-20261030T133512-7f3a",
        ] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn the_oldest_runs_go_and_the_current_one_stays() {
        let folder = tempfile::tempdir().unwrap();
        for index in 0..=4 {
            for second in 0..10 {
                let id = format!("3fa9c1d27b04-2026103{index}T00000{second}-0000");
                std::fs::create_dir(folder.path().join(id)).unwrap();
            }
        }
        std::fs::create_dir(folder.path().join("keep-me")).unwrap();
        std::fs::write(folder.path().join("3fa9c1d27b04-20261030T000000-0001"), "").unwrap();
        let current = "3fa9c1d27b04-20200101T000000-0000";
        std::fs::create_dir(folder.path().join(current)).unwrap();

        assert_eq!(prune(folder.path(), current), 0);

        let mut left: Vec<String> = std::fs::read_dir(folder.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert!(left.contains(&current.to_string()));
        assert!(left.contains(&"keep-me".to_string()));
        assert!(left.contains(&"3fa9c1d27b04-20261030T000000-0001".to_string()));
        assert!(!left.contains(&"3fa9c1d27b04-20261030T000000-0000".to_string()));
        assert!(left.contains(&"3fa9c1d27b04-20261034T000009-0000".to_string()));
        let runs = left.iter().filter(|name| is_valid_id(name)).count();
        let stray_file = 1;
        assert_eq!(runs, KEEP_RUNS + stray_file);
    }

    #[test]
    fn a_stray_folder_name_cannot_stop_a_run() {
        let folder = tempfile::tempdir().unwrap();
        let root = schematic(folder.path());
        let first = create(folder.path(), &root).unwrap();
        let project = first.dir.parent().unwrap();
        let hash = first.id.split('-').next().unwrap();
        std::fs::create_dir(project.join(format!("{hash}-20261030\u{5e9}T1234-0000"))).unwrap();
        assert!(create(folder.path(), &root).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_run_that_will_not_go_is_counted_and_stops_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempfile::tempdir().unwrap();
        let stuck = folder.path().join("3fa9c1d27b04-20000101T000000-0000");
        std::fs::create_dir_all(stuck.join("inner/deep")).unwrap();
        let inner = stuck.join("inner");
        std::fs::set_permissions(&inner, std::fs::Permissions::from_mode(0o555)).unwrap();
        for second in 0..KEEP_RUNS {
            let id = format!("3fa9c1d27b04-20261030T{second:06}-0000");
            std::fs::create_dir(folder.path().join(id)).unwrap();
        }
        let kept = prune(folder.path(), "3fa9c1d27b04-20261030T000000-0000");
        std::fs::set_permissions(&inner, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(kept, 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_never_followed_or_removed() {
        let folder = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("precious"), "").unwrap();
        let link = folder.path().join("3fa9c1d27b04-20000101T000000-0000");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        for second in 0..KEEP_RUNS + 2 {
            let id = format!("3fa9c1d27b04-20261030T{second:06}-0000");
            std::fs::create_dir(folder.path().join(id)).unwrap();
        }
        prune(folder.path(), "3fa9c1d27b04-20261030T000000-0000");
        assert!(outside.path().join("precious").exists());
        assert!(link.symlink_metadata().is_ok());
    }
}
