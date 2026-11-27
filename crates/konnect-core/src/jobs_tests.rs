//! Tests of the background runs (Row AP2). The work is a future the test controls;
//! nothing here starts a process.

use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::oneshot;

const KIND: JobKind = JobKind {
    folder: "test-runs",
    tool: "test_tool",
};

const FIRST: &str = "1700000000000-0123456789ab";
const SECOND: &str = "1700000000001-0123456789ab";

/// A progress the test sets.
#[derive(Default)]
struct Count(AtomicUsize);

impl Progress for Count {
    fn snapshot(&self) -> Value {
        json!({ "count": self.0.load(Ordering::SeqCst) })
    }
}

struct Run {
    _dir: tempfile::TempDir,
    state: PathBuf,
    board: PathBuf,
}

fn run_place() -> Run {
    let dir = tempfile::tempdir().unwrap();
    let state = std::fs::canonicalize(dir.path()).unwrap();
    let board = state.join("board.kicad_pcb");
    Run {
        _dir: dir,
        state,
        board,
    }
}

/// Claim `board` for `id`, make its folder and start `work` there.
fn start<W>(jobs: &Jobs, run: &Run, board: &Path, id: &str, progress: Arc<dyn Progress>, work: W)
where
    W: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let claim = claim_board(&run.state, &KIND, board, id)
        .unwrap()
        .expect("the board is free");
    let folder = run_folder_of(&run.state, &KIND, id);
    std::fs::create_dir_all(&folder).unwrap();
    jobs.start(&KIND, claim, board, folder, progress, work)
        .unwrap();
}

const LONG: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread")]
async fn a_second_start_on_a_board_is_refused_and_names_the_running_id() {
    let run = run_place();
    let jobs = Jobs::default();
    let (release, released) = oneshot::channel::<()>();
    start(
        &jobs,
        &run,
        &run.board,
        FIRST,
        Arc::new(Count::default()),
        async move {
            released.await.ok();
            Ok(json!({ "answer": 42 }))
        },
    );
    let busy = claim_board(&run.state, &KIND, &run.board, SECOND)
        .unwrap()
        .expect_err("one run per board");
    assert_eq!(busy.run_id.as_deref(), Some(FIRST));
    // Another board is free, and a claim that is dropped frees its board.
    let other = run.state.join("other.kicad_pcb");
    drop(
        claim_board(&run.state, &KIND, &other, SECOND)
            .unwrap()
            .unwrap(),
    );
    assert!(claim_board(&run.state, &KIND, &other, SECOND)
        .unwrap()
        .is_ok());

    release.send(()).unwrap();
    let report = jobs.report(&run.state, &KIND, FIRST, LONG).await;
    assert!(matches!(report, Report::Done { .. }), "{report:?}");
    assert!(
        claim_board(&run.state, &KIND, &run.board, SECOND)
            .unwrap()
            .is_ok(),
        "the board is free once its run has ended"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_run_is_read_by_its_id_after_a_restart() {
    let run = run_place();
    let jobs = Jobs::default();
    let progress = Arc::new(Count::default());
    progress.0.store(7, Ordering::SeqCst);
    start(&jobs, &run, &run.board, FIRST, progress, async {
        Ok(json!({ "status": "planned", "apply_with": { "tool": "x" } }))
    });
    let Report::Done { record, result } = jobs.report(&run.state, &KIND, FIRST, LONG).await else {
        panic!("the run ends");
    };
    assert_eq!(
        result,
        json!({ "status": "planned", "apply_with": { "tool": "x" } })
    );
    assert_eq!(record.state, RunState::Done);
    assert_eq!(record.progress, json!({ "count": 7 }));
    assert!(record.finished_unix_ms.is_some());
    assert_eq!(record.board, run.board.display().to_string());

    // A new registry is a restarted server: nothing in memory, the folder is read.
    let restarted = Jobs::default();
    let again = restarted
        .report(&run.state, &KIND, FIRST, Duration::ZERO)
        .await;
    assert_eq!(again, Report::Done { record, result });
    let folder = run_folder_of(&run.state, &KIND, FIRST);
    assert!(folder.join(RESULT_FILE).is_file());
    assert!(
        !folder.join("run.json.tmp").exists(),
        "no half-written record"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_run_reports_its_live_progress_and_a_wait_ends_with_the_run() {
    let run = run_place();
    let jobs = Jobs::default();
    let progress = Arc::new(Count::default());
    let (release, released) = oneshot::channel::<()>();
    start(
        &jobs,
        &run,
        &run.board,
        FIRST,
        progress.clone(),
        async move {
            released.await.ok();
            Ok(json!({ "done": true }))
        },
    );
    progress.0.store(3, Ordering::SeqCst);
    let Report::Running {
        record,
        elapsed_s,
        here,
    } = jobs
        .report(&run.state, &KIND, FIRST, Duration::from_millis(200))
        .await
    else {
        panic!("still running");
    };
    assert!(here);
    assert_eq!(
        record.progress,
        json!({ "count": 3 }),
        "live, not as written"
    );
    assert_eq!(record.state, RunState::Running);
    assert!(elapsed_s >= 0.2, "{elapsed_s}");

    // A long wait answers as soon as the run ends, not when the wait is over.
    let waited = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        release.send(()).unwrap();
    });
    let report = jobs.report(&run.state, &KIND, FIRST, MAX_WAIT).await;
    assert!(matches!(report, Report::Done { .. }), "{report:?}");
    assert!(waited.elapsed() < Duration::from_secs(20));
}

#[tokio::test(start_paused = true)]
async fn a_running_run_writes_its_progress_to_its_folder_for_other_processes() {
    let run = run_place();
    let jobs = Jobs::default();
    let progress = Arc::new(Count::default());
    let (release, released) = oneshot::channel::<()>();
    start(
        &jobs,
        &run,
        &run.board,
        FIRST,
        progress.clone(),
        async move {
            released.await.ok();
            Ok(json!({}))
        },
    );
    let folder = run_folder_of(&run.state, &KIND, FIRST);
    assert_eq!(
        read_record(&folder).unwrap().progress,
        json!({ "count": 0 })
    );
    progress.0.store(5, Ordering::SeqCst);
    tokio::time::sleep(PERSIST_EVERY + Duration::from_secs(1)).await;
    let written = read_record(&folder).unwrap();
    assert_eq!(written.progress, json!({ "count": 5 }));
    assert_eq!(written.state, RunState::Running);
    // Read as another process would (its own registry): running, by a held lock.
    let other = Jobs::default()
        .report(&run.state, &KIND, FIRST, Duration::ZERO)
        .await;
    let Report::Running { record, here, .. } = other else {
        panic!("{other:?}");
    };
    assert!(!here);
    assert_eq!(record.progress, json!({ "count": 5 }));
    release.send(()).unwrap();
    let report = jobs.report(&run.state, &KIND, FIRST, LONG).await;
    assert!(matches!(report, Report::Done { .. }), "{report:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_errs_or_panics_is_failed_with_its_reason() {
    let run = run_place();
    let jobs = Jobs::default();
    start(
        &jobs,
        &run,
        &run.board,
        FIRST,
        Arc::new(Count::default()),
        async { Err(anyhow::anyhow!("the input board cannot be read")) },
    );
    let other = run.state.join("other.kicad_pcb");
    start(
        &jobs,
        &run,
        &other,
        SECOND,
        Arc::new(Count::default()),
        async { explode() },
    );
    for (id, reason) in [
        (FIRST, "the input board cannot be read"),
        (SECOND, "the run panicked: boom"),
    ] {
        let report = jobs.report(&run.state, &KIND, id, LONG).await;
        let Report::Failed { record } = &report else {
            panic!("{report:?}");
        };
        assert_eq!(record.error.as_deref(), Some(reason));
        assert!(!run_folder_of(&run.state, &KIND, id)
            .join(RESULT_FILE)
            .exists());
        // And read back the same after a restart.
        assert_eq!(
            Jobs::default()
                .report(&run.state, &KIND, id, Duration::ZERO)
                .await,
            report
        );
    }
    assert!(claim_board(&run.state, &KIND, &run.board, SECOND)
        .unwrap()
        .is_ok());
}

fn explode() -> anyhow::Result<Value> {
    panic!("boom")
}

/// A `running` record as a server that stopped leaves it.
fn left_running(run: &Run, id: &str) -> PathBuf {
    let folder = run_folder_of(&run.state, &KIND, id);
    std::fs::create_dir_all(&folder).unwrap();
    let record = RunRecord {
        run_id: id.to_string(),
        tool: KIND.tool.to_string(),
        board: run.board.display().to_string(),
        state: RunState::Running,
        pid: 1,
        started_unix_ms: now_ms() - 5_000,
        finished_unix_ms: None,
        progress: json!({ "count": 2 }),
        error: None,
    };
    write_json(&folder.join(RUN_FILE), &record).unwrap();
    std::fs::write(folder.join(LOCK_FILE), "").unwrap();
    folder
}

#[tokio::test]
async fn a_running_record_with_a_free_lock_is_interrupted_and_a_held_one_is_running() {
    let run = run_place();
    let folder = left_running(&run, FIRST);
    let jobs = Jobs::default();
    let report = jobs.report(&run.state, &KIND, FIRST, Duration::ZERO).await;
    assert!(matches!(report, Report::Interrupted { .. }), "{report:?}");

    let lock = File::open(folder.join(LOCK_FILE)).unwrap();
    lock.try_lock().unwrap();
    let report = jobs.report(&run.state, &KIND, FIRST, Duration::ZERO).await;
    let Report::Running {
        here, elapsed_s, ..
    } = report
    else {
        panic!("{report:?}");
    };
    assert!(!here);
    assert!(elapsed_s >= 5.0, "{elapsed_s}");
    drop(lock);
    let report = jobs.report(&run.state, &KIND, FIRST, Duration::ZERO).await;
    assert!(matches!(report, Report::Interrupted { .. }), "{report:?}");
}

#[tokio::test]
async fn only_a_run_id_of_this_kind_in_its_folder_is_read() {
    let run = run_place();
    let jobs = Jobs::default();
    for id in [
        "",
        "../1700000000000-0123456789ab",
        "1700000000000-0123456789AB",
        "1700000000000-0123456789a",
        "1700000000000_0123456789ab",
        "123456789012345678901-0123456789ab",
        "1700000000000-0123456789ab/..",
    ] {
        assert!(!valid_run_id(id), "{id}");
        let report = jobs.report(&run.state, &KIND, id, Duration::ZERO).await;
        let Report::Unknown { why } = report else {
            panic!("{id}: {report:?}");
        };
        assert!(why.contains("not a run id"), "{why}");
    }
    assert!(valid_run_id(FIRST));

    let report = jobs.report(&run.state, &KIND, FIRST, Duration::ZERO).await;
    assert!(
        matches!(&report, Report::Unknown { why } if why.contains("no run folder")),
        "{report:?}"
    );
    let folder = run_folder_of(&run.state, &KIND, FIRST);
    std::fs::create_dir_all(&folder).unwrap();
    let report = jobs.report(&run.state, &KIND, FIRST, Duration::ZERO).await;
    assert!(
        matches!(&report, Report::Unknown { why } if why.contains("holds no run record")),
        "{report:?}"
    );
    left_running(&run, FIRST);
    let other_kind = JobKind {
        folder: KIND.folder,
        tool: "another_tool",
    };
    let report = jobs
        .report(&run.state, &other_kind, FIRST, Duration::ZERO)
        .await;
    assert!(
        matches!(&report, Report::Unknown { why } if why.contains("is a run of test_tool")),
        "{report:?}"
    );
    let mut done = read_record(&folder).unwrap();
    done.state = RunState::Done;
    write_json(&folder.join(RUN_FILE), &done).unwrap();
    let report = jobs.report(&run.state, &KIND, FIRST, Duration::ZERO).await;
    assert!(
        matches!(&report, Report::Unknown { why } if why.contains("cannot be read")),
        "a done record without its result: {report:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claim_never_names_a_run_that_has_ended() {
    let run = run_place();
    let jobs = Jobs::default();
    start(
        &jobs,
        &run,
        &run.board,
        FIRST,
        Arc::new(Count::default()),
        async { Ok(json!({})) },
    );
    assert!(matches!(
        jobs.report(&run.state, &KIND, FIRST, LONG).await,
        Report::Done { .. }
    ));
    // The body still names FIRST; a holder that has not written its own yet is
    // "starting", not the run that ended.
    let held = claim_board(&run.state, &KIND, &run.board, SECOND);
    let held = held.unwrap().unwrap();
    let digest = Sha256::digest(run.board.to_string_lossy().as_bytes());
    let key: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    let body = run
        .state
        .join(KIND.folder)
        .join(BOARDS_FOLDER)
        .join(format!("{key}.json"));
    write_json(
        &body,
        &ClaimBody {
            run_id: FIRST.to_string(),
            board: run.board.display().to_string(),
            pid: 1,
        },
    )
    .unwrap();
    let busy = claim_board(&run.state, &KIND, &run.board, "1700000000002-0123456789ab")
        .unwrap()
        .unwrap_err();
    assert_eq!(busy.run_id, None);
    drop(held);
}
