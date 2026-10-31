//! Where runs live: `<state root>/simulation/<project hash>/<run id>/`.
//!
//! Outside the project, because raw files can reach hundreds of MB and projects
//! may sit in synced folders. The id starts with the project hash, so finding a
//! run later needs no scan.

use super::kicad_netlist::{ElementEntry, NetEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const KEEP_RUNS: usize = 20;
const HASH_CHARS: usize = 12;
const STAMP_CHARS: usize = 15;
const SUFFIX_CHARS: usize = 4;
const CREATE_ATTEMPTS: usize = 8;
/// What a finished run keeps beside its data, so a later call can measure it
/// without the circuit.
pub(crate) const RECORD_FILE: &str = "run.json";
const RECORD_VERSION: u32 = 1;
/// Far above any real record, which holds the net and element names of one design.
const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;
/// The worker's `status.json` is a few hundred bytes.
pub(crate) const MAX_STATUS_BYTES: u64 = 1024 * 1024;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunState {
    Complete,
    Partial,
}

impl RunState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }
}

/// Why a partial run stopped, as `run_simulation` reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stopped {
    pub reason: String,
    pub at_point: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

/// What `run_simulation` knew about a finished run that its data files do not
/// say. Only runs with data are recorded: a run that failed returned an error and
/// has nothing to measure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunRecord {
    pub version: u32,
    /// The canonical root sheet, so the inputs can be checked again.
    pub schematic: PathBuf,
    /// The analysis as `Analysis::record` writes it.
    pub analysis: Value,
    pub run_status: RunState,
    pub stopped: Option<Stopped>,
    pub simulated_circuit: String,
    pub coverage_ratio: f64,
    pub inputs_hash: String,
    /// Model files the deck inlined, which the hash covers.
    pub inlined_files: Vec<PathBuf>,
    /// Kept to resolve a net or reference to the engine's name.
    pub nets: Vec<NetEntry>,
    pub elements: Vec<ElementEntry>,
    /// What the run said about itself that a later measurement is read with: the
    /// engine's messages, a missing first point, a sweep that ended short.
    #[serde(default)]
    pub warnings: Vec<Value>,
}

impl RunRecord {
    pub(crate) fn new(fields: RecordFields) -> Self {
        Self {
            version: RECORD_VERSION,
            schematic: fields.schematic,
            analysis: fields.analysis,
            run_status: fields.run_status,
            stopped: fields.stopped,
            simulated_circuit: fields.simulated_circuit,
            coverage_ratio: fields.coverage_ratio,
            inputs_hash: fields.inputs_hash,
            inlined_files: fields.inlined_files,
            nets: fields.nets,
            elements: fields.elements,
            warnings: fields.warnings,
        }
    }

    pub(crate) fn analysis_kind(&self) -> &str {
        self.analysis["kind"].as_str().unwrap_or_default()
    }
}

/// Everything a record holds apart from its version.
pub(crate) struct RecordFields {
    pub schematic: PathBuf,
    pub analysis: Value,
    pub run_status: RunState,
    pub stopped: Option<Stopped>,
    pub simulated_circuit: String,
    pub coverage_ratio: f64,
    pub inputs_hash: String,
    pub inlined_files: Vec<PathBuf>,
    pub nets: Vec<NetEntry>,
    pub elements: Vec<ElementEntry>,
    pub warnings: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RecordError {
    #[error("the run folder holds no record")]
    Missing,
    #[error("the run record cannot be used: {0}")]
    Unusable(String),
}

/// Write beside the final name and rename, so a reader never sees half a record.
/// A name that is already there, such as a link, is replaced and not followed.
pub(crate) fn write_record(dir: &Path, record: &RunRecord) -> std::io::Result<()> {
    let partial = dir.join(format!("{RECORD_FILE}.partial"));
    std::fs::write(&partial, serde_json::to_vec(record)?)?;
    std::fs::rename(&partial, dir.join(RECORD_FILE)).inspect_err(|_| {
        std::fs::remove_file(&partial).ok();
    })
}

/// A file's bytes, if it is a plain file of at most `cap` bytes. A link, a device
/// or a pipe in the store is never read: a record or a status file is only ever
/// written there as a plain file.
pub(crate) fn read_plain(path: &Path, cap: u64) -> std::io::Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::other("it is not a plain file"));
    }
    if meta.len() > cap {
        return Err(std::io::Error::other(format!("it is {} bytes", meta.len())));
    }
    std::fs::read(path)
}

pub(crate) fn read_record(dir: &Path) -> Result<RunRecord, RecordError> {
    let bytes = match read_plain(&dir.join(RECORD_FILE), MAX_RECORD_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(RecordError::Missing)
        }
        Err(error) => return Err(RecordError::Unusable(error.to_string())),
    };
    // The parser's own message can quote the text it choked on.
    let record: RunRecord = serde_json::from_slice(&bytes).map_err(|error| {
        RecordError::Unusable(format!(
            "it does not parse, at line {} column {}",
            error.line(),
            error.column()
        ))
    })?;
    match record.version == RECORD_VERSION {
        true => Ok(record),
        false => Err(RecordError::Unusable(format!(
            "it is version {}, and this server reads version {RECORD_VERSION}",
            record.version
        ))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocateError {
    /// Not the shape of a run id. Nothing was looked up.
    InvalidId,
    NotFound,
}

/// The folder of a run, found from its id alone: the id starts with the project
/// hash. The id is checked first, because it becomes two path components.
pub(crate) fn locate(state_root: &Path, id: &str) -> Result<PathBuf, LocateError> {
    if !is_valid_id(id) {
        return Err(LocateError::InvalidId);
    }
    let dir = state_root
        .join("simulation")
        .join(&id[..HASH_CHARS])
        .join(id);
    // `symlink_metadata` does not follow a link, so a link in the store is no run.
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => Ok(dir),
        _ => Err(LocateError::NotFound),
    }
}

/// The ids of a project's runs, newest first. Ids sort by their time stamp, and
/// two made in one second by their random end.
pub(crate) fn list(state_root: &Path, schematic: &Path) -> std::io::Result<Vec<String>> {
    let project = state_root.join("simulation").join(project_hash(schematic)?);
    let entries = match std::fs::read_dir(&project) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_valid_id(name))
        .collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    Ok(ids)
}

/// `2026-10-30T13:35:12Z` from a valid id.
pub(crate) fn created_at(id: &str) -> Option<String> {
    let stamp = id.get(HASH_CHARS + 1..HASH_CHARS + 1 + STAMP_CHARS)?;
    let (date, time) = stamp.split_once('T')?;
    Some(format!(
        "{}-{}-{}T{}:{}:{}Z",
        date.get(..4)?,
        date.get(4..6)?,
        date.get(6..8)?,
        time.get(..2)?,
        time.get(2..4)?,
        time.get(4..6)?
    ))
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

    const ID: &str = "3fa9c1d27b04-20261030T133512-7f3a";

    fn record() -> RunRecord {
        RunRecord::new(RecordFields {
            schematic: PathBuf::from("/p/top.kicad_sch"),
            analysis: serde_json::json!({ "kind": "transient", "stop_time_seconds": 1.0 }),
            run_status: RunState::Partial,
            stopped: Some(Stopped {
                reason: "timeout".to_string(),
                at_point: 7,
                node: None,
            }),
            simulated_circuit: "modified".to_string(),
            coverage_ratio: 0.5,
            inputs_hash: "abc".to_string(),
            inlined_files: vec![PathBuf::from("/p/m.lib")],
            nets: Vec::new(),
            elements: Vec::new(),
            warnings: vec![serde_json::json!({ "code": "engine_messages" })],
        })
    }

    fn store_with(id: &str) -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("simulation").join(&id[..12]).join(id);
        std::fs::create_dir_all(&dir).unwrap();
        (root, dir)
    }

    #[test]
    fn a_run_is_found_from_its_id_alone() {
        let (root, dir) = store_with(ID);
        assert_eq!(locate(root.path(), ID), Ok(dir));
        assert_eq!(
            locate(root.path(), "3fa9c1d27b04-20261030T133512-0000"),
            Err(LocateError::NotFound)
        );
        // The same id under another project hash is not this run.
        assert_eq!(
            locate(root.path(), "aaaaaaaaaaaa-20261030T133512-7f3a"),
            Err(LocateError::NotFound)
        );
    }

    #[test]
    fn nothing_but_an_exact_id_is_looked_up() {
        let (root, _) = store_with(ID);
        let outside = root.path().join("simulation").join("secret");
        std::fs::create_dir_all(&outside).unwrap();
        for bad in [
            "",
            "..",
            ".",
            "../simulation",
            "../../etc/passwd",
            "/etc/passwd",
            "3fa9c1d27b04/../3fa9c1d27b04-20261030T133512-7f3a",
            "3fa9c1d27b04-20261030T133512-7f3a/",
            "3fa9c1d27b04-20261030T133512-7f3a/..",
            "3fa9c1d27b04-20261030T133512-7f3a\0",
            "3fa9c1d27b04-20261030T133512-7f3a ",
            "..\\3fa9c1d27b04-20261030T133512-7f3a",
        ] {
            assert_eq!(
                locate(root.path(), bad),
                Err(LocateError::InvalidId),
                "{bad:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_or_a_file_in_the_store_is_not_a_run() {
        let (root, dir) = store_with(ID);
        std::fs::remove_dir(&dir).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), &dir).unwrap();
        assert_eq!(locate(root.path(), ID), Err(LocateError::NotFound));
        std::fs::remove_file(&dir).unwrap();
        std::fs::write(&dir, "").unwrap();
        assert_eq!(locate(root.path(), ID), Err(LocateError::NotFound));
    }

    #[test]
    fn runs_are_listed_newest_first_and_strays_are_left_out() {
        let folder = tempfile::tempdir().unwrap();
        let sheet = schematic(folder.path());
        let state = tempfile::tempdir().unwrap();
        assert_eq!(list(state.path(), &sheet).unwrap(), Vec::<String>::new());
        let first = create(state.path(), &sheet).unwrap();
        let project = first.dir.parent().unwrap().to_path_buf();
        let hash = first.id[..HASH_CHARS].to_string();
        for stamp in [
            "20250101T000000-0001",
            "20260101T000000-0000",
            "20250101T000000-0002",
        ] {
            std::fs::create_dir(project.join(format!("{hash}-{stamp}"))).unwrap();
        }
        std::fs::create_dir(project.join("stray")).unwrap();
        std::fs::write(project.join(format!("{hash}-20270101T000000-0000")), "").unwrap();
        let listed = list(state.path(), &sheet).unwrap();
        // The one made by `create` carries today's date, and is newest.
        assert_eq!(listed[0], first.id);
        assert_eq!(
            &listed[1..],
            [
                format!("{hash}-20260101T000000-0000"),
                format!("{hash}-20250101T000000-0002"),
                format!("{hash}-20250101T000000-0001"),
            ]
        );
    }

    #[test]
    fn a_record_comes_back_as_it_was_written() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record()).unwrap();
        assert_eq!(read_record(dir.path()), Ok(record()));
    }

    #[test]
    fn a_record_that_is_missing_or_cannot_be_trusted_says_which() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_record(dir.path()), Err(RecordError::Missing));
        let path = dir.path().join(RECORD_FILE);
        std::fs::write(&path, "{ nope").unwrap();
        assert!(matches!(
            read_record(dir.path()),
            Err(RecordError::Unusable(_))
        ));
        let mut wrong = serde_json::to_value(record()).unwrap();
        wrong["version"] = serde_json::json!(RECORD_VERSION + 1);
        std::fs::write(&path, wrong.to_string()).unwrap();
        let Err(RecordError::Unusable(why)) = read_record(dir.path()) else {
            panic!("a record of another version must not be read")
        };
        assert!(why.contains("version"), "{why}");
        // A folder where the file belongs is not a record either.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(
            read_record(dir.path()),
            Err(RecordError::Unusable(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_record_that_is_a_link_is_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write_record(outside.path(), &record()).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join(RECORD_FILE),
            dir.path().join(RECORD_FILE),
        )
        .unwrap();
        assert!(matches!(
            read_record(dir.path()),
            Err(RecordError::Unusable(_))
        ));
    }

    #[test]
    fn a_record_past_the_size_limit_is_not_read() {
        // Valid JSON padded with spaces, which would parse if it were read.
        let dir = tempfile::tempdir().unwrap();
        let mut text = serde_json::to_string(&record()).unwrap();
        text.push_str(&" ".repeat(MAX_RECORD_BYTES as usize));
        std::fs::write(dir.path().join(RECORD_FILE), text).unwrap();
        let Err(RecordError::Unusable(why)) = read_record(dir.path()) else {
            panic!("a record past the limit must not be read")
        };
        assert!(why.contains("bytes"), "{why}");
        // At the limit it is read.
        let mut text = serde_json::to_string(&record()).unwrap();
        let room = MAX_RECORD_BYTES as usize - text.len();
        text.push_str(&" ".repeat(room));
        std::fs::write(dir.path().join(RECORD_FILE), text).unwrap();
        assert_eq!(read_record(dir.path()), Ok(record()));
    }

    #[test]
    fn a_record_error_never_quotes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(RECORD_FILE),
            "{\"version\": \"secret-text\"}",
        )
        .unwrap();
        let Err(RecordError::Unusable(why)) = read_record(dir.path()) else {
            panic!("not a record")
        };
        assert!(!why.contains("secret-text"), "{why}");
        assert!(why.contains("line 1"), "{why}");
    }

    #[test]
    fn only_a_small_plain_file_is_read_for_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("status.json");
        std::fs::write(&file, "{}").unwrap();
        assert_eq!(read_plain(&file, 2).unwrap(), b"{}");
        assert!(read_plain(&file, 1).is_err());
        assert!(read_plain(dir.path(), 100).is_err(), "a folder");
        assert!(read_plain(&dir.path().join("none"), 100).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_or_a_link_is_never_read_for_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        // A read of this would block for ever, so a test that fails hangs.
        assert!(read_plain(&fifo, 100).is_err());
        let real = dir.path().join("real");
        std::fs::write(&real, "{}").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(read_plain(&link, 100).is_err());
    }

    #[test]
    fn a_record_is_replaced_whole_and_leaves_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record()).unwrap();
        let mut newer = record();
        newer.coverage_ratio = 1.0;
        write_record(dir.path(), &newer).unwrap();
        assert_eq!(read_record(dir.path()), Ok(newer));
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [RECORD_FILE]);
    }

    #[cfg(unix)]
    #[test]
    fn a_record_does_not_follow_a_link_that_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("precious");
        std::fs::write(&target, "mine").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(RECORD_FILE)).unwrap();
        write_record(dir.path(), &record()).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "mine");
        assert_eq!(read_record(dir.path()), Ok(record()));
    }

    #[test]
    fn a_stopped_node_that_is_unknown_is_left_out() {
        let stopped = Stopped {
            reason: "timeout".to_string(),
            at_point: 3,
            node: None,
        };
        assert_eq!(
            serde_json::to_value(&stopped).unwrap(),
            serde_json::json!({ "reason": "timeout", "at_point": 3 })
        );
        let read: Stopped = serde_json::from_str(r#"{"reason":"timeout","at_point":3}"#).unwrap();
        assert_eq!(read, stopped);
    }

    #[test]
    fn the_time_in_an_id_reads_as_a_date() {
        assert_eq!(created_at(ID).as_deref(), Some("2026-10-30T13:35:12Z"));
        assert_eq!(created_at("short"), None);
    }
}
