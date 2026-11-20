//! Tests of the fastroute module. No real fastroute is needed: where a binary is
//! needed, a small C program (`FAKE_C`, compiled once with the system `cc`) stands
//! in for it. A compiled program is used rather than a shell script because the
//! sandbox profile (decision 13) allows exactly one program to start: a `#!/bin/sh`
//! script would need `/bin/sh` and then `/bin/bash` (sh dispatches to it) to start
//! as well, and the profile refuses both. The fake reads `control.txt` from its own
//! folder, so one build serves every test.

use super::*;
use serde_json::json;
use std::collections::HashSet;

fn hints(clearance: Option<i64>, retry: Option<i64>) -> RoutingHints {
    RoutingHints {
        copper_edge_clearance_um: clearance,
        retry_width_um: retry,
        retry_off_reason: None,
    }
}

fn pass(name: &'static str, neck: f64, optimizer: bool, initial: Option<&str>) -> Pass {
    Pass {
        name,
        neck_width_um: neck,
        optimizer,
        initial_session: initial.map(PathBuf::from),
    }
}

fn args_of(pass: &Pass, hints: &RoutingHints, amps: Option<&Path>) -> Vec<String> {
    args_with(pass, hints, CallFiles { amps, pairs: None })
}

fn args_with(pass: &Pass, hints: &RoutingHints, files: CallFiles<'_>) -> Vec<String> {
    pass_arguments(
        Path::new("/d/in.dsn"),
        Path::new("/d/out.ses"),
        Path::new("/d/out.json"),
        hints,
        files,
        pass,
    )
}

fn key(token: &str) -> &str {
    token.split('=').next().unwrap()
}

fn value<'a>(args: &'a [String], prefix: &str) -> Option<&'a str> {
    args.iter().find_map(|token| token.strip_prefix(prefix))
}

// ---- pass_arguments -------------------------------------------------------

#[test]
fn pass_arguments_follow_the_documented_order() {
    let args = args_of(
        &pass("full", 0.0, true, None),
        &hints(Some(500), Some(120)),
        None,
    );
    let mut expected: Vec<String> = [
        "-de",
        "/d/in.dsn",
        "-do",
        "/d/out.ses",
        "--report=/d/out.json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    expected.extend(PRESET.iter().map(|s| s.to_string()));
    expected.extend(
        [
            "--router.optimizer.enabled=true",
            "--router.neck_width_um=0",
            "--router.copper_to_edge_clearance_um=500",
            "--router.autorouter.max_threads=2",
            "--router.optimizer.max_threads=2",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    expected.extend(CANDIDATE_SWITCHES.iter().map(|s| s.to_string()));
    assert_eq!(args, expected);
}

#[test]
fn every_pass_kind_sends_each_setting_once_and_nothing_forbidden() {
    let amps = PathBuf::from("/d/amps.txt");
    let passes = [
        pass("full", 0.0, true, None),
        pass("fallback", 120.0, false, Some("/d/with space/full.ses")),
        pass("polish", 0.0, true, Some("/d/with space/fallback.ses")),
    ];
    for pass in &passes {
        for (amps, pairs) in [
            (None, None),
            (Some(amps.as_path()), None),
            (None, Some(Path::new("/d/pairs.txt"))),
            (Some(amps.as_path()), Some(Path::new("/d/pairs.txt"))),
        ] {
            let args = args_with(
                pass,
                &hints(Some(250), Some(120)),
                CallFiles { amps, pairs },
            );
            let mut seen = HashSet::new();
            for token in &args {
                assert!(
                    seen.insert(key(token).to_string()),
                    "{}: {token} repeats a setting: {args:?}",
                    pass.name
                );
                for forbidden in ["--live", "--multi-start", "--max-time", "--diagnose"] {
                    assert!(
                        !token.starts_with(forbidden),
                        "{}: forbidden {token}",
                        pass.name
                    );
                }
            }
            for switch in CANDIDATE_SWITCHES.iter().chain(PRESET.iter()) {
                assert_eq!(
                    args.iter()
                        .filter(|token| token.as_str() == *switch)
                        .count(),
                    1,
                    "{}: {switch}",
                    pass.name
                );
            }
            assert_eq!(
                value(&args, "--router.optimizer.enabled="),
                Some(if pass.optimizer { "true" } else { "false" })
            );
        }
    }
}

#[test]
fn initial_session_is_one_token_and_only_when_the_pass_has_one() {
    let full = args_of(&pass("full", 0.0, true, None), &hints(None, None), None);
    assert!(value(&full, "--initial-session=").is_none());
    assert!(!full.iter().any(|token| token == "--initial-session"));
    let fallback = args_of(
        &pass("fallback", 120.0, false, Some("/d/with space/full.ses")),
        &hints(None, None),
        None,
    );
    assert_eq!(
        value(&fallback, "--initial-session="),
        Some("/d/with space/full.ses")
    );
    assert_eq!(
        fallback.last().unwrap(),
        "--initial-session=/d/with space/full.ses"
    );
    assert!(!fallback
        .iter()
        .any(|token| token == "/d/with space/full.ses"));
}

#[test]
fn amps_switch_is_present_only_when_given() {
    let none = args_of(&pass("full", 0.0, true, None), &hints(None, None), None);
    assert!(value(&none, "--x.par_via_amps=").is_none());
    let amps = Path::new("/d/amps.txt");
    let some = args_of(
        &pass("fallback", 120.0, false, Some("/d/full.ses")),
        &hints(None, None),
        Some(amps),
    );
    assert_eq!(value(&some, "--x.par_via_amps="), Some("/d/amps.txt"));
    // After the candidate switches, before the initial session.
    let at = |prefix: &str| some.iter().position(|t| t.starts_with(prefix)).unwrap();
    assert!(at("--x.par_via_each_a=") < at("--x.par_via_amps="));
    assert!(at("--x.par_via_amps=") < at("--initial-session="));
}

/// `--pairs` is a per-call input: it is sent only when the call has a pairs file, as
/// one token, after the amps switch and before the initial session, and it is not a
/// candidate switch (those go on every pass of every call).
#[test]
fn pairs_switch_is_present_only_when_given() {
    let none = args_of(&pass("full", 0.0, true, None), &hints(None, None), None);
    assert!(!none.iter().any(|token| token.starts_with("--pairs")));
    let pairs = Path::new("/d/with space/pairs.txt");
    let amps = Path::new("/d/amps.txt");
    let some = args_with(
        &pass("fallback", 120.0, false, Some("/d/full.ses")),
        &hints(None, None),
        CallFiles {
            amps: Some(amps),
            pairs: Some(pairs),
        },
    );
    assert_eq!(value(&some, "--pairs="), Some("/d/with space/pairs.txt"));
    assert_eq!(
        some.iter()
            .filter(|token| token.starts_with("--pairs"))
            .count(),
        1
    );
    assert!(!some.iter().any(|token| token == "/d/with space/pairs.txt"));
    let at = |prefix: &str| some.iter().position(|t| t.starts_with(prefix)).unwrap();
    assert!(at("--x.par_via_amps=") < at("--pairs="));
    assert!(at("--pairs=") < at("--initial-session="));
    // Without amps the pairs file still goes, and the amps switch does not.
    let only = args_with(
        &pass("full", 0.0, true, None),
        &hints(None, None),
        CallFiles {
            amps: None,
            pairs: Some(pairs),
        },
    );
    assert_eq!(value(&only, "--pairs="), Some("/d/with space/pairs.txt"));
    assert!(value(&only, "--x.par_via_amps=").is_none());
    // The file is never one of the fixed switches, and the rest of the line is the
    // same with and without it.
    assert!(!CANDIDATE_SWITCHES
        .iter()
        .any(|switch| switch.starts_with("--pairs")));
    let without: Vec<&String> = only
        .iter()
        .filter(|token| !token.starts_with("--pairs"))
        .collect();
    let plain = args_of(&pass("full", 0.0, true, None), &hints(None, None), None);
    assert_eq!(without, plain.iter().collect::<Vec<_>>());
}

#[test]
fn edge_clearance_is_absent_without_a_hint_and_250_is_sent_as_250_001() {
    let pass = pass("full", 0.0, true, None);
    let absent = args_of(&pass, &hints(None, None), None);
    assert!(!absent
        .iter()
        .any(|token| token.starts_with("--router.copper_to_edge_clearance_um")));
    for (input, expected) in [
        (250, "250.001"),
        (500, "500"),
        (249, "249"),
        (251, "251"),
        (0, "0"),
    ] {
        let args = args_of(&pass, &hints(Some(input), None), None);
        assert_eq!(
            value(&args, "--router.copper_to_edge_clearance_um="),
            Some(expected),
            "clearance {input}"
        );
    }
}

#[test]
fn numbers_drop_a_trailing_point_zero() {
    assert_eq!(number(0.0), "0");
    assert_eq!(number(-0.0), "0");
    assert_eq!(number(200.0), "200");
    assert_eq!(number(250.001), "250.001");
    assert_eq!(number(12.5), "12.5");
    assert_eq!(number(0.8975), "0.8975");
    for (neck, expected) in [(0.0, "0"), (200.0, "200"), (250.001, "250.001")] {
        let args = args_of(
            &pass("fallback", neck, false, None),
            &hints(None, None),
            None,
        );
        assert_eq!(
            value(&args, "--router.neck_width_um="),
            Some(expected),
            "neck {neck}"
        );
    }
}

// ---- amps_file_text -------------------------------------------------------

fn manifest(rows: serde_json::Value) -> Value {
    json!({ "intent_widths": rows })
}

#[test]
fn amps_file_lists_routed_nets_with_a_current_sorted_once_each() {
    let rows = json!([
        { "net": "VBUS", "current_a": 2.0, "status": "widened" },
        { "net": "GND", "current_a": 0.5, "status": "not_widened" },
        { "net": "NOCURRENT", "status": "widened" },
        { "net": "SKIPPED", "current_a": 3.0, "status": "not_routed" },
        { "net": "GND", "current_a": 9.0, "status": "not_routed" },
        { "net": "VBUS", "current_a": 5.0, "status": "widened" },
        { "net": "BAD\nNET", "current_a": 1.0, "status": "widened" },
        { "net": "TEXT", "current_a": "2", "status": "widened" },
        { "net": "A", "current_a": 1.5 }
    ]);
    let expected = "1.5 A\n0.5 GND\n5 VBUS\n";
    assert_eq!(
        amps_file_text(&manifest(rows.clone())).as_deref(),
        Some(expected)
    );
    // The largest current wins whichever row comes first.
    let mut reversed = rows.as_array().unwrap().clone();
    reversed.reverse();
    assert_eq!(
        amps_file_text(&manifest(json!(reversed))).as_deref(),
        Some(expected)
    );
}

#[test]
fn amps_file_is_none_without_a_usable_row() {
    assert_eq!(amps_file_text(&json!({})), None);
    assert_eq!(amps_file_text(&manifest(json!([]))), None);
    assert_eq!(
        amps_file_text(&manifest(json!([
            { "net": "X", "current_a": 1.0, "status": "not_routed" },
            { "net": "Y", "status": "widened" },
            { "net": "Z\r", "current_a": 1.0 },
        ]))),
        None
    );
}

// ---- sandbox_profile ------------------------------------------------------

#[test]
fn sandbox_profile_has_exactly_the_documented_text() {
    let indent = " ".repeat(19);
    let expected = format!(
        "(version 1)\n\
         (allow default)\n\
         (deny network*)\n\
         (deny file-write*)\n\
         (allow file-write* (subpath \"/tmp/run folder\") (literal \"/dev/null\") (literal \"/dev/zero\")\n\
         {indent}(literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))\n\
         (deny process-exec)\n\
         (allow process-exec (literal \"/opt/fastroute/fastroute\"))\n"
    );
    assert_eq!(
        sandbox_profile(
            Path::new("/tmp/run folder"),
            Path::new("/opt/fastroute/fastroute")
        )
        .unwrap(),
        expected
    );
}

#[test]
fn sandbox_profile_refuses_unsafe_or_relative_paths() {
    let good = Path::new("/tmp/run");
    let binary = Path::new("/opt/fastroute/fastroute");
    // The honest cases pass, including characters that need no quoting.
    for ok in [
        "/tmp/run (1)",
        "/tmp/ru n",
        "/tmp/r\u{fc}n",
        "/tmp/\u{5e8}\u{5d9}\u{5e5}",
    ] {
        assert!(sandbox_profile(Path::new(ok), binary).is_ok(), "{ok}");
        assert!(sandbox_profile(good, Path::new(ok)).is_ok(), "{ok}");
    }
    assert!(sandbox_profile(good, binary).is_ok());
    // Each refusal, in the folder and in the binary position.
    for bad in [
        "run",
        "./run",
        "/tmp/a\"b",
        "/tmp/a\\b",
        "/tmp/a\nb",
        "/tmp/a\rb",
        "/tmp/a\tb",
        "/tmp/a\u{7}b",
        "/tmp/a\u{7f}b",
        "/tmp/a\0b",
    ] {
        let error = sandbox_profile(Path::new(bad), binary).expect_err(bad);
        assert!(
            error.to_string().contains("sandbox profile"),
            "{bad}: {error}"
        );
        assert!(
            sandbox_profile(good, Path::new(bad)).is_err(),
            "binary {bad}"
        );
    }
}

#[test]
fn tail_keeps_the_end() {
    assert_eq!(tail("abc", 5), "abc");
    assert_eq!(tail("abcdef", 3), "def");
}

// ---- on a system that cannot isolate a run ---------------------------------

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn locate_is_unsupported_off_macos() {
    let scratch = tempfile::tempdir().unwrap();
    let located = locate(
        Some(Path::new("/bin/ls")),
        None,
        None,
        None,
        None,
        scratch.path(),
    )
    .await;
    assert!(
        matches!(located, Located::Unsupported { .. }),
        "{located:?}"
    );
}

// ---- sandboxed runs (macOS) -------------------------------------------------

#[cfg(target_os = "macos")]
mod sandboxed {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;

    /// The stand-in for fastroute. Behaviour comes from `control.txt` beside the
    /// executable (`key=value` lines).
    const FAKE_C: &str = r##"#include <arpa/inet.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

static char control[16384];

static int cfg(const char *key, char *val, size_t n) {
    size_t kl = strlen(key);
    char *p = control;
    while (*p) {
        char *eol = strchr(p, '\n');
        size_t len = eol ? (size_t)(eol - p) : strlen(p);
        if (len > kl && strncmp(p, key, kl) == 0 && p[kl] == '=') {
            size_t vl = len - kl - 1;
            if (vl >= n) vl = n - 1;
            memcpy(val, p + kl + 1, vl);
            val[vl] = 0;
            return 1;
        }
        if (!eol) break;
        p = eol + 1;
    }
    return 0;
}

static int cfg_int(const char *key, int def) {
    char v[64];
    return cfg(key, v, sizeof v) ? atoi(v) : def;
}

static void escape_attempts(const char *sesdir) {
    char path[4400];
    snprintf(path, sizeof path, "%s/escape.txt", sesdir);
    FILE *e = fopen(path, "w");
    if (!e) return;
    snprintf(path, sizeof path, "%s/../outside.txt", sesdir);
    int fd = open(path, O_CREAT | O_WRONLY, 0644);
    fprintf(e, "write_outside=%s\n", fd >= 0 ? "ok" : "denied");
    if (fd >= 0) close(fd);
    int s = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons((unsigned short)cfg_int("port", 9));
    inet_pton(AF_INET, "127.0.0.1", &a.sin_addr);
    int r = s >= 0 ? connect(s, (struct sockaddr *)&a, sizeof a) : -1;
    fprintf(e, "network=%s\n", r == 0 ? "ok" : "denied");
    if (s >= 0) close(s);
    pid_t pid = fork();
    if (pid == 0) {
        execl("/bin/ls", "ls", (char *)0);
        _exit(127);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    fprintf(e, "exec=%s\n", (WIFEXITED(st) && WEXITSTATUS(st) == 127) ? "denied" : "ok");
    fclose(e);
}

int main(int argc, char **argv) {
    char dir[4096];
    snprintf(dir, sizeof dir, "%s", argv[0]);
    char *slash = strrchr(dir, '/');
    if (slash) *slash = 0; else strcpy(dir, ".");
    char cpath[4200];
    snprintf(cpath, sizeof cpath, "%s/control.txt", dir);
    FILE *f = fopen(cpath, "r");
    if (f) {
        size_t n = fread(control, 1, sizeof control - 1, f);
        control[n] = 0;
        fclose(f);
    }
    char mode[256] = "candidate";
    cfg("mode", mode, sizeof mode);
    if (strcmp(mode, "sleep") == 0) {
        sleep(30);
        return 0;
    }
    const char *dsn = NULL, *ses = NULL, *report = NULL, *pairs = NULL;
    int version = 0;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--version") == 0) version = 1;
        else if (strcmp(argv[i], "-de") == 0 && i + 1 < argc) dsn = argv[++i];
        else if (strcmp(argv[i], "-do") == 0 && i + 1 < argc) ses = argv[++i];
        else if (strncmp(argv[i], "--report=", 9) == 0) report = argv[i] + 9;
        else if (strncmp(argv[i], "--pairs=", 8) == 0) pairs = argv[i] + 8;
    }
    if (version) {
        char text[256] = "fastroute 0.1.14";
        cfg("version", text, sizeof text);
        puts(text);
        return 0;
    }
    if (!ses) {
        if (strcmp(mode, "ignore_experimental") == 0) {
            printf("INFO fastroute starting\n");
            printf("No such file or directory (os error 2): %s\n", dsn ? dsn : "?");
            return 1;
        }
        if (strcmp(mode, "names_switch") == 0) {
            char sw[256] = "--x.par_vias_net";
            cfg("switch", sw, sizeof sw);
            printf("fastroute: unknown experimental switch %s\n", sw);
            if (cfg_int("also_sentinel", 0))
                printf("fastroute: unknown experimental switch --x.konnect_probe_unknown\n");
            return 2;
        }
        for (int i = 1; i < argc; i++) {
            if (strcmp(argv[i], "--x.konnect_probe_unknown") == 0) {
                printf("fastroute: unknown experimental switch %s\n", argv[i]);
                return 2;
            }
        }
        printf("no sentinel\n");
        return 0;
    }
    char sesdir[4096];
    snprintf(sesdir, sizeof sesdir, "%s", ses);
    slash = strrchr(sesdir, '/');
    if (slash) *slash = 0; else strcpy(sesdir, ".");
    char pass[128];
    const char *base = strrchr(ses, '/');
    base = base ? base + 1 : ses;
    snprintf(pass, sizeof pass, "%s", base);
    char *dot = strrchr(pass, '.');
    if (dot) *dot = 0;

    char path[4400];
    snprintf(path, sizeof path, "%s.env", ses);
    FILE *e = fopen(path, "w");
    if (e) {
        for (char **v = environ; *v; v++) fprintf(e, "%s\n", *v);
        fclose(e);
    }
    snprintf(path, sizeof path, "%s.pid", ses);
    e = fopen(path, "w");
    if (e) {
        fprintf(e, "%d\n", (int)getpid());
        fclose(e);
    }
    snprintf(path, sizeof path, "%s.args", ses);
    e = fopen(path, "w");
    if (e) {
        for (int i = 1; i < argc; i++) fprintf(e, "%s\n", argv[i]);
        fclose(e);
    }

    if (pairs) {
        // What the run could read of its pairs file, beside the session.
        snprintf(path, sizeof path, "%s.pairs", ses);
        FILE *out = fopen(path, "w");
        FILE *in = fopen(pairs, "r");
        if (out) {
            if (in) {
                char buf[1024];
                size_t n;
                while ((n = fread(buf, 1, sizeof buf, in)) > 0) fwrite(buf, 1, n, out);
            } else {
                fputs("UNREADABLE\n", out);
            }
            fclose(out);
        }
        if (in) fclose(in);
    }

    char key[300], val[1100];
    snprintf(key, sizeof key, "sleep.%s", pass);
    if (cfg(key, val, sizeof val)) {
        sleep(30);
        return 0;
    }
    snprintf(key, sizeof key, "fail.%s", pass);
    if (cfg(key, val, sizeof val)) {
        fprintf(stderr, "fake failure marker for %s\n", pass);
        return 3;
    }
    if (cfg("exit", val, sizeof val)) {
        fprintf(stderr, "boom marker\n");
        return atoi(val);
    }
    if (cfg_int("escape", 0)) escape_attempts(sesdir);

    snprintf(key, sizeof key, "unrouted.%s", pass);
    int unrouted = cfg_int(key, cfg_int("unrouted", 0));
    int violations = cfg_int("violations", 0);
    if (report && !cfg_int("noreport", 0)) {
        FILE *r = fopen(report, "w");
        if (r) {
            if (cfg("report", val, sizeof val)) {
                fputs(val, r);
            } else {
                fputs("{\"unrouted\":[", r);
                for (int i = 0; i < unrouted; i++) fputs(i ? ",{}" : "{}", r);
                fputs("],\"clearance_violations\":[", r);
                for (int i = 0; i < violations; i++) fputs(i ? ",{}" : "{}", r);
                fputs("]}", r);
            }
            fclose(r);
        }
    }
    if (!cfg_int("nosession", 0)) {
        FILE *s = fopen(ses, "w");
        if (s) {
            if (!cfg_int("empty_session", 0)) fputs("(session fake)\n", s);
            fclose(s);
        }
    }
    return 0;
}
"##;

    /// The fake, compiled once per source text into a folder of the system temp
    /// folder named by the source's hash; the copy of a test is made from it.
    fn fake_template() -> &'static Path {
        static FAKE: OnceLock<PathBuf> = OnceLock::new();
        FAKE.get_or_init(|| {
            let hash = format!("{:x}", Sha256::digest(FAKE_C.as_bytes()));
            let dir = std::env::temp_dir().join(format!("konnect-fastroute-fake-{}", &hash[..16]));
            let built = dir.join("fake");
            if built.is_file() {
                return built;
            }
            std::fs::create_dir_all(&dir).unwrap();
            let pid = std::process::id();
            let source = dir.join(format!("fake-{pid}.c"));
            let staged = dir.join(format!("fake-{pid}"));
            std::fs::write(&source, FAKE_C).unwrap();
            let status = std::process::Command::new("/usr/bin/cc")
                .arg("-O0")
                .arg("-o")
                .arg(&staged)
                .arg(&source)
                .status()
                .expect("/usr/bin/cc is needed to build the fake fastroute");
            assert!(status.success(), "the fake fastroute did not compile");
            std::fs::rename(&staged, &built).unwrap();
            let _ = std::fs::remove_file(&source);
            built
        })
    }

    /// One test's folder: a copy of the fake in `bin/` with its control file.
    struct Fixture {
        dir: tempfile::TempDir,
        binary: PathBuf,
    }

    impl Fixture {
        fn new(control: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let binary = bin.join("fastroute");
            std::fs::copy(fake_template(), &binary).unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::write(bin.join("control.txt"), control).unwrap();
            Self { dir, binary }
        }
        fn path(&self) -> &Path {
            self.dir.path()
        }
        fn run_dir(&self) -> PathBuf {
            self.path().join("run")
        }
        fn scratch(&self) -> PathBuf {
            self.path().join("scratch")
        }
        fn dsn(&self) -> PathBuf {
            let dsn = self.path().join("design.dsn");
            std::fs::write(&dsn, "(pcb test)").unwrap();
            dsn
        }
        fn binary(&self) -> Binary {
            Binary {
                path: self.binary.canonicalize().unwrap(),
                sha256: "test".to_string(),
                version: "fastroute 0.1.14".to_string(),
                source: "argument",
            }
        }
    }

    fn soon(seconds: u64) -> Instant {
        Instant::now() + Duration::from_secs(seconds)
    }

    fn alive(pid: &str) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn failed_text<T: std::fmt::Debug>(result: std::result::Result<T, PassError>) -> String {
        match result {
            Err(PassError::Failed(error)) => format!("{error:#}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    async fn run_full(
        fixture: &Fixture,
        hints: &RoutingHints,
        seconds: u64,
    ) -> std::result::Result<PassOutcome, PassError> {
        run_pass(
            &fixture.binary(),
            &fixture.run_dir(),
            &fixture.dsn(),
            hints,
            CallFiles::default(),
            pass("full", 0.0, true, None),
            soon(seconds),
        )
        .await
    }

    // ---- probe --------------------------------------------------------------

    #[tokio::test]
    async fn probe_accepts_the_candidates_answers_with_sha_and_version() {
        let fixture = Fixture::new("mode=candidate\n");
        let binary = probe(&fixture.binary, "argument", &fixture.scratch())
            .await
            .unwrap();
        assert_eq!(binary.version, "fastroute 0.1.14");
        assert_eq!(binary.source, "argument");
        assert_eq!(binary.path, fixture.binary.canonicalize().unwrap());
        let bytes = std::fs::read(&fixture.binary).unwrap();
        assert_eq!(binary.sha256, format!("{:x}", Sha256::digest(&bytes)));
        assert_eq!(binary.sha256.len(), 64);
        // The probe ran under the profile of its folder.
        let scratch = fixture.scratch().canonicalize().unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.join("probe.sb")).unwrap(),
            sandbox_profile(&scratch, &binary.path).unwrap()
        );
    }

    #[tokio::test]
    async fn probe_refuses_a_build_that_ignores_experimental_switches() {
        let fixture = Fixture::new("mode=ignore_experimental\n");
        let error = probe(&fixture.binary, "argument", &fixture.scratch())
            .await
            .unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("probe step 2"), "{text}");
        assert!(text.contains("No such file or directory"), "{text}");
    }

    #[tokio::test]
    async fn probe_refuses_a_build_that_names_a_candidate_switch_as_unknown() {
        // Names the missing switch only, as a build that stops at its first unknown.
        let fixture = Fixture::new("mode=names_switch\nswitch=--x.par_vias_net\n");
        let text = format!(
            "{:#}",
            probe(&fixture.binary, "argument", &fixture.scratch())
                .await
                .unwrap_err()
        );
        assert!(text.contains("--x.par_vias_net"), "{text}");
        // Names the missing switch and also the sentinel: only the "no other switch
        // is unknown" guard can refuse this one.
        let fixture = Fixture::new("mode=names_switch\nswitch=--x.par_vias_net\nalso_sentinel=1\n");
        let text = format!(
            "{:#}",
            probe(&fixture.binary, "argument", &fixture.scratch())
                .await
                .unwrap_err()
        );
        assert!(text.contains("names --x.par_vias_net"), "{text}");
    }

    #[tokio::test]
    async fn probe_refuses_a_version_line_that_is_not_fastroutes() {
        for version in ["not fastroute 1.0", "fastroute", " fastroute 0.1.14", ""] {
            let fixture = Fixture::new(&format!("mode=candidate\nversion={version}\n"));
            let text = format!(
                "{:#}",
                probe(&fixture.binary, "argument", &fixture.scratch())
                    .await
                    .unwrap_err()
            );
            assert!(text.contains("probe step 1"), "{version:?}: {text}");
        }
        // The honest version line passes the same step.
        let fixture = Fixture::new("mode=candidate\nversion=fastroute 9.9.9\n");
        let binary = probe(&fixture.binary, "argument", &fixture.scratch())
            .await
            .unwrap();
        assert_eq!(binary.version, "fastroute 9.9.9");
    }

    #[tokio::test]
    async fn probe_kills_a_build_that_does_not_answer() {
        let fixture = Fixture::new("mode=sleep\n");
        let started = std::time::Instant::now();
        let error = probe(&fixture.binary, "argument", &fixture.scratch())
            .await
            .unwrap_err();
        let elapsed = started.elapsed();
        let text = format!("{error:#}");
        assert!(text.contains("did not answer"), "{text}");
        assert!(text.contains("probe step 1"), "{text}");
        assert!(
            elapsed >= PROBE_TIMEOUT - Duration::from_secs(1),
            "{elapsed:?}"
        );
        assert!(
            elapsed < PROBE_TIMEOUT + Duration::from_secs(4),
            "{elapsed:?}"
        );
    }

    // ---- locate -------------------------------------------------------------

    fn plain_file(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "not a build").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    fn found(located: Located) -> Binary {
        match located {
            Located::Found(binary) => binary,
            other => panic!("expected Found, got {other:?}"),
        }
    }

    fn refused(located: Located) -> (PathBuf, &'static str, String) {
        match located {
            Located::Refused {
                path,
                source,
                reason,
            } => (path, source, reason),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn locate_prefers_argument_over_environment_over_config_over_default() {
        let a = Fixture::new("mode=candidate\n");
        let b = Fixture::new("mode=candidate\n");
        let c = Fixture::new("mode=candidate\n");
        let home = tempfile::tempdir().unwrap();
        let default = home.path().join(DEFAULT_PATH);
        std::fs::create_dir_all(default.parent().unwrap()).unwrap();
        std::fs::copy(&a.binary, &default).unwrap();
        std::fs::copy(
            a.binary.parent().unwrap().join("control.txt"),
            default.parent().unwrap().join("control.txt"),
        )
        .unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let s = |name: &str| scratch.path().join(name);

        let all = locate(
            Some(&a.binary),
            Some(b.binary.to_str().unwrap()),
            Some(&c.binary),
            None,
            Some(home.path()),
            &s("1"),
        )
        .await;
        let binary = found(all);
        assert_eq!(binary.source, "argument");
        assert_eq!(binary.path, a.binary.canonicalize().unwrap());

        let binary = found(
            locate(
                None,
                Some(b.binary.to_str().unwrap()),
                Some(&c.binary),
                None,
                Some(home.path()),
                &s("2"),
            )
            .await,
        );
        assert_eq!(binary.source, "environment");
        assert_eq!(binary.path, b.binary.canonicalize().unwrap());

        let binary = found(
            locate(
                None,
                None,
                Some(&c.binary),
                None,
                Some(home.path()),
                &s("3"),
            )
            .await,
        );
        assert_eq!(binary.source, "user_config");
        assert_eq!(binary.path, c.binary.canonicalize().unwrap());

        let binary = found(locate(None, None, None, None, Some(home.path()), &s("4")).await);
        assert_eq!(binary.source, "default");
        assert_eq!(binary.path, default.canonicalize().unwrap());
        assert_eq!(binary.version, "fastroute 0.1.14");
        assert_eq!(binary.sha256.len(), 64);
    }

    #[tokio::test]
    async fn a_later_source_is_not_tried_after_an_earlier_one_is_refused() {
        let good = Fixture::new("mode=candidate\n");
        let dir = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let s = |name: &str| scratch.path().join(name);
        let not_executable = plain_file(dir.path(), "plain", 0o644);
        let directory = dir.path().join("a_folder");
        std::fs::create_dir_all(&directory).unwrap();
        let missing = dir.path().join("missing");

        // The argument is refused although the environment and the config are good.
        let (path, source, reason) = refused(
            locate(
                Some(&not_executable),
                Some(good.binary.to_str().unwrap()),
                Some(&good.binary),
                None,
                None,
                &s("1"),
            )
            .await,
        );
        assert_eq!(source, "argument");
        assert_eq!(path, not_executable);
        assert!(reason.contains("not executable"), "{reason}");

        let (_, source, reason) = refused(
            locate(
                None,
                Some(directory.to_str().unwrap()),
                Some(&good.binary),
                None,
                None,
                &s("2"),
            )
            .await,
        );
        assert_eq!(source, "environment");
        assert!(reason.contains("not a regular file"), "{reason}");

        let (_, source, _) =
            refused(locate(None, None, Some(&not_executable), None, None, &s("3")).await);
        assert_eq!(source, "user_config");

        let (_, source, reason) = refused(
            locate(
                Some(&missing),
                None,
                Some(&good.binary),
                None,
                None,
                &s("4"),
            )
            .await,
        );
        assert_eq!(source, "argument");
        assert!(reason.contains("cannot be read"), "{reason}");

        // The honest cases: each of these sources, alone, is accepted.
        assert_eq!(
            found(locate(Some(&good.binary), None, None, None, None, &s("5")).await).source,
            "argument"
        );
        assert_eq!(
            found(
                locate(
                    None,
                    Some(good.binary.to_str().unwrap()),
                    None,
                    None,
                    None,
                    &s("6")
                )
                .await
            )
            .source,
            "environment"
        );
        assert_eq!(
            found(locate(None, None, Some(&good.binary), None, None, &s("7")).await).source,
            "user_config"
        );
    }

    #[tokio::test]
    async fn an_empty_environment_value_names_nothing() {
        let good = Fixture::new("mode=candidate\n");
        let scratch = tempfile::tempdir().unwrap();
        let binary = found(
            locate(
                None,
                Some(""),
                Some(&good.binary),
                None,
                None,
                scratch.path(),
            )
            .await,
        );
        assert_eq!(binary.source, "user_config");
    }

    #[tokio::test]
    async fn locate_reports_the_default_place_when_nothing_is_named() {
        let home = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        match locate(None, None, None, None, Some(home.path()), scratch.path()).await {
            Located::NotConfigured { default_path } => {
                assert_eq!(default_path, home.path().join(DEFAULT_PATH));
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
        match locate(None, None, None, None, None, scratch.path()).await {
            Located::NotConfigured { default_path } => {
                assert!(default_path.ends_with(DEFAULT_PATH), "{default_path:?}");
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
        // A directory or a non-executable file at the default place is refused,
        // not "not configured".
        let default = home.path().join(DEFAULT_PATH);
        std::fs::create_dir_all(&default).unwrap();
        let (path, source, reason) =
            refused(locate(None, None, None, None, Some(home.path()), scratch.path()).await);
        assert_eq!((path, source), (default.clone(), "default"));
        assert!(reason.contains("not a regular file"), "{reason}");
        std::fs::remove_dir_all(&default).unwrap();
        std::fs::write(&default, "x").unwrap();
        std::fs::set_permissions(&default, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (_, source, reason) =
            refused(locate(None, None, None, None, Some(home.path()), scratch.path()).await);
        assert_eq!(source, "default");
        assert!(reason.contains("not executable"), "{reason}");
    }

    #[tokio::test]
    async fn locate_refuses_a_build_that_fails_the_probe() {
        let fixture = Fixture::new("mode=ignore_experimental\n");
        let scratch = tempfile::tempdir().unwrap();
        let (path, source, reason) = refused(
            locate(
                Some(&fixture.binary),
                None,
                None,
                None,
                None,
                scratch.path(),
            )
            .await,
        );
        assert_eq!(source, "argument");
        assert_eq!(path, fixture.binary.canonicalize().unwrap());
        assert!(reason.contains("probe step 2"), "{reason}");
    }

    #[tokio::test]
    async fn locate_follows_a_symbolic_link_to_its_target() {
        let fixture = Fixture::new("mode=candidate\n");
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&fixture.binary, &link).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let binary = found(locate(Some(&link), None, None, None, None, scratch.path()).await);
        assert_eq!(binary.path, fixture.binary.canonicalize().unwrap());
    }

    /// 1 Kopper: a binary packaged next to the Konnect executable comes after the
    /// user config and before the default place. A broken one is refused, never
    /// passed over for the default build; a missing one leaves it to the default.
    #[tokio::test]
    async fn locate_takes_a_bundled_binary_after_the_config_and_before_the_default() {
        let bundled = Fixture::new("mode=candidate\n");
        let config = Fixture::new("mode=candidate\n");
        let home = tempfile::tempdir().unwrap();
        let default = home.path().join(DEFAULT_PATH);
        std::fs::create_dir_all(default.parent().unwrap()).unwrap();
        std::fs::copy(&config.binary, &default).unwrap();
        std::fs::copy(
            config.binary.parent().unwrap().join("control.txt"),
            default.parent().unwrap().join("control.txt"),
        )
        .unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let s = |name: &str| scratch.path().join(name);
        let home = Some(home.path());

        let binary = found(
            locate(
                None,
                None,
                Some(&config.binary),
                Some(&bundled.binary),
                home,
                &s("1"),
            )
            .await,
        );
        assert_eq!(binary.source, "user_config");

        // A good bundled build wins over a good default one.
        let binary = found(locate(None, None, None, Some(&bundled.binary), home, &s("2")).await);
        assert_eq!(binary.source, "bundled");
        assert_eq!(binary.path, bundled.binary.canonicalize().unwrap());

        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join(BUNDLED_NAME);
        let binary = found(locate(None, None, None, Some(&missing), home, &s("3")).await);
        assert_eq!(binary.source, "default");

        let broken = plain_file(dir.path(), BUNDLED_NAME, 0o600);
        let (path, source, reason) =
            refused(locate(None, None, None, Some(&broken), home, &s("4")).await);
        assert_eq!((path, source), (broken, "bundled"));
        assert!(reason.contains("not executable"), "{reason}");
    }

    /// Only an executable inside a package (its manifest one folder up) has a
    /// bundled place; a link finds the package it points into.
    #[test]
    fn the_bundled_place_is_beside_the_resolved_executable_of_a_package() {
        let package = tempfile::tempdir().unwrap();
        let bin = package.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = plain_file(&bin, "konnect", 0o755);
        assert_eq!(bundled_path(&exe), None, "no manifest: not a package");
        std::fs::write(package.path().join(PACKAGE_MANIFEST), "{}").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let link = elsewhere.path().join("konnect");
        std::os::unix::fs::symlink(&exe, &link).unwrap();
        let expected = bin.canonicalize().unwrap().join(BUNDLED_NAME);
        assert_eq!(bundled_path(&exe), Some(expected.clone()));
        assert_eq!(bundled_path(&link), Some(expected));
        assert_eq!(bundled_path(&elsewhere.path().join("absent")), None);
        // A folder named like the manifest is not one.
        std::fs::remove_file(package.path().join(PACKAGE_MANIFEST)).unwrap();
        std::fs::create_dir(package.path().join(PACKAGE_MANIFEST)).unwrap();
        assert_eq!(bundled_path(&exe), None);
    }

    // ---- run_pass -----------------------------------------------------------

    #[tokio::test]
    async fn a_pass_reports_the_unrouted_count_of_its_report() {
        let fixture = Fixture::new("unrouted=2\nviolations=0\n");
        let outcome = run_full(&fixture, &hints(Some(250), None), 30)
            .await
            .unwrap();
        assert_eq!(outcome.unrouted, 2);
        assert_eq!(outcome.clearance_violations, 0);
        assert_eq!(outcome.ses, fixture.run_dir().join("full.ses"));
        assert!(std::fs::metadata(&outcome.ses).unwrap().len() > 0);
        assert!(outcome.seconds >= 0.0);
        assert!(fixture.run_dir().join("home").is_dir());
        assert!(fixture.run_dir().join("tmp").is_dir());
        assert!(fixture.run_dir().join("fastroute.log").is_file());
        // The profile is the one of the real path of the folder.
        let real = fixture.run_dir().canonicalize().unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.run_dir().join("profile.sb")).unwrap(),
            sandbox_profile(&real, &fixture.binary().path).unwrap()
        );
        let counted = Fixture::new("unrouted=0\nviolations=2\n");
        let outcome = run_full(&counted, &hints(None, None), 30).await.unwrap();
        assert_eq!((outcome.unrouted, outcome.clearance_violations), (0, 2));
    }

    #[tokio::test]
    async fn a_pass_gets_the_documented_command_line() {
        let fixture = Fixture::new("unrouted=0\n");
        let amps = fixture.path().join("amps.txt");
        std::fs::write(&amps, "2 VBUS\n").unwrap();
        let hints = hints(Some(250), Some(120));
        let pass = pass("fallback", 120.0, false, Some("/x/full.ses"));
        run_pass(
            &fixture.binary(),
            &fixture.run_dir(),
            &fixture.dsn(),
            &hints,
            CallFiles {
                amps: Some(&amps),
                pairs: None,
            },
            pass.clone(),
            soon(30),
        )
        .await
        .unwrap();
        let seen: Vec<String> =
            std::fs::read_to_string(fixture.run_dir().join("fallback.ses.args"))
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect();
        let expected = pass_arguments(
            &fixture.dsn(),
            &fixture.run_dir().join("fallback.ses"),
            &fixture.run_dir().join("fallback.json"),
            &hints,
            CallFiles {
                amps: Some(&amps),
                pairs: None,
            },
            &pass,
        );
        assert_eq!(seen, expected);
    }

    #[tokio::test]
    async fn a_pass_sees_only_home_tmpdir_and_path() {
        // Variables a leaking child would show; set in this process only.
        std::env::set_var("FASTROUTE_LEAK_TEST", "1");
        std::env::set_var("FREEROUTING__LEAK_TEST", "1");
        std::env::set_var("KONNECT_LEAK_TEST", "1");
        let fixture = Fixture::new("unrouted=0\n");
        run_full(&fixture, &hints(None, None), 30).await.unwrap();
        let dump = std::fs::read_to_string(fixture.run_dir().join("full.ses.env")).unwrap();
        let mut names: Vec<&str> = dump.lines().map(key).collect();
        names.sort_unstable();
        assert_eq!(names, ["HOME", "PATH", "TMPDIR"], "{dump}");
        assert!(!dump.contains("FASTROUTE_"), "{dump}");
        assert!(!dump.contains("FREEROUTING__"), "{dump}");
        let real = fixture.run_dir().canonicalize().unwrap();
        let get = |name: &str| {
            dump.lines()
                .find_map(|line| line.strip_prefix(&format!("{name}=")))
                .unwrap()
                .to_string()
        };
        assert_eq!(get("HOME"), real.join("home").display().to_string());
        assert_eq!(get("TMPDIR"), real.join("tmp").display().to_string());
        assert_eq!(get("PATH"), "/usr/bin:/bin");
    }

    #[tokio::test]
    async fn a_non_zero_exit_fails_with_the_log_tail() {
        let fixture = Fixture::new("exit=7\n");
        let text = failed_text(run_full(&fixture, &hints(None, None), 30).await);
        assert!(text.contains("exited with"), "{text}");
        assert!(text.contains("boom marker"), "{text}");
    }

    #[tokio::test]
    async fn a_missing_or_empty_session_fails() {
        let fixture = Fixture::new("nosession=1\n");
        let text = failed_text(run_full(&fixture, &hints(None, None), 30).await);
        assert!(text.contains("no session"), "{text}");
        let fixture = Fixture::new("empty_session=1\n");
        let text = failed_text(run_full(&fixture, &hints(None, None), 30).await);
        assert!(text.contains("no session"), "{text}");
    }

    #[tokio::test]
    async fn a_report_that_does_not_read_fails() {
        for (control, expect) in [
            (
                "report={\"unrouted\":3,\"clearance_violations\":[]}\n",
                "unrouted is not a list",
            ),
            (
                "report={\"unrouted\":[],\"clearance_violations\":\"none\"}\n",
                "clearance_violations is not a list",
            ),
            (
                "report={\"unrouted\":[]}\n",
                "clearance_violations is not a list",
            ),
            ("report=this is not json\n", "is not JSON"),
            ("noreport=1\n", "could not be read"),
        ] {
            let fixture = Fixture::new(control);
            let text = failed_text(run_full(&fixture, &hints(None, None), 30).await);
            assert!(text.contains(expect), "{control}: {text}");
        }
        // The honest report of the same shape reads.
        let fixture = Fixture::new("report={\"unrouted\":[{},{}],\"clearance_violations\":[]}\n");
        assert_eq!(
            run_full(&fixture, &hints(None, None), 30)
                .await
                .unwrap()
                .unrouted,
            2
        );
    }

    #[tokio::test]
    async fn a_pass_past_its_deadline_is_killed_and_waited_for() {
        let fixture = Fixture::new("sleep.full=1\n");
        let started = std::time::Instant::now();
        // 6 s, not 2: the test reads the pid the fake writes once it has started, and
        // under the full suite's load the sandboxed start alone took more than 2 s.
        let result = run_full(&fixture, &hints(None, None), 6).await;
        let elapsed = started.elapsed();
        match result {
            Err(PassError::Killed { seconds }) => assert!(seconds >= 5.5, "{seconds}"),
            other => panic!("expected Killed, got {other:?}"),
        }
        assert!(elapsed < Duration::from_secs(15), "{elapsed:?}");
        let pid = std::fs::read_to_string(fixture.run_dir().join("full.ses.pid"))
            .expect("the fake wrote its pid before it slept");
        assert!(!alive(&pid), "the child {pid} survived the kill");
    }

    #[tokio::test]
    async fn dropping_the_future_kills_the_run() {
        let fixture = Fixture::new("sleep.full=1\n");
        let binary = fixture.binary();
        let dsn = fixture.dsn();
        let run_dir = fixture.run_dir();
        let routing_hints = hints(None, None);
        let run = run_pass(
            &binary,
            &run_dir,
            &dsn,
            &routing_hints,
            CallFiles::default(),
            pass("full", 0.0, true, None),
            soon(120),
        );
        // Drive the run until the fake has written its pid (it is then sleeping), then
        // drop the future. A fixed wait raced the fake's start under parallel load.
        let pid_file = run_dir.join("full.ses.pid");
        let started = async {
            for _ in 0..200 {
                if std::fs::read_to_string(&pid_file).is_ok_and(|pid| !pid.trim().is_empty()) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("the fake never wrote its pid");
        };
        tokio::select! {
            _ = run => panic!("the sleeping fake ended before it was dropped"),
            () = started => {}
        }
        let pid =
            std::fs::read_to_string(&pid_file).expect("the fake wrote its pid before it slept");
        for _ in 0..50 {
            if !alive(&pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the child {pid} outlived the dropped future");
    }

    /// The fake tries to write beside the run folder, to reach a socket that is
    /// listening on loopback, and to start `/bin/ls`, and records what happened.
    fn escape_results(path: &Path) -> String {
        std::fs::read_to_string(path.join("escape.txt")).unwrap()
    }

    #[tokio::test]
    async fn the_sandbox_denies_outside_writes_the_network_and_other_programs() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let fixture = Fixture::new(&format!("escape=1\nport={port}\nunrouted=0\n"));

        // Under the profile: all three are denied, and the run still finishes.
        run_full(&fixture, &hints(None, None), 30).await.unwrap();
        assert_eq!(
            escape_results(&fixture.run_dir()),
            "write_outside=denied\nnetwork=denied\nexec=denied\n"
        );
        assert!(!fixture.path().join("outside.txt").exists());

        // The negative control: the same program, run without the profile, does all
        // three, so the denials above are the profile's doing.
        let control = fixture.path().join("control");
        std::fs::create_dir_all(&control).unwrap();
        let status = std::process::Command::new(&fixture.binary)
            .arg("-de")
            .arg(fixture.dsn())
            .arg("-do")
            .arg(control.join("full.ses"))
            .arg(format!("--report={}", control.join("full.json").display()))
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            escape_results(&control),
            "write_outside=ok\nnetwork=ok\nexec=ok\n"
        );
        assert!(fixture.path().join("outside.txt").exists());
        drop(listener);
    }

    // ---- route_held ---------------------------------------------------------

    fn floor_hints() -> RoutingHints {
        RoutingHints {
            copper_edge_clearance_um: Some(250),
            retry_width_um: Some(120),
            retry_off_reason: Some("different floors"),
        }
    }

    async fn hold(
        fixture: &Fixture,
        hints: &RoutingHints,
        fallback: WidthFallback,
        seconds: u64,
    ) -> std::result::Result<Held, PassError> {
        route_held(
            &fixture.binary(),
            &fixture.path().join("held"),
            &fixture.dsn(),
            hints,
            CallFiles::default(),
            fallback,
            soon(seconds),
        )
        .await
    }

    fn row(held: &Held, index: usize) -> (&str, &str, Option<&str>) {
        let row = &held.passes[index];
        (
            row["name"].as_str().unwrap(),
            row["outcome"].as_str().unwrap(),
            row["reason"].as_str(),
        )
    }

    fn pass_args(fixture: &Fixture, name: &str) -> Vec<String> {
        std::fs::read_to_string(
            fixture
                .path()
                .join("held")
                .join(name)
                .join(format!("{name}.ses.args")),
        )
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
    }

    fn ran(fixture: &Fixture, name: &str) -> bool {
        fixture.path().join("held").join(name).exists()
    }

    #[tokio::test]
    async fn a_complete_full_pass_ends_the_run() {
        let fixture = Fixture::new("unrouted=0\n");
        let held = hold(&fixture, &floor_hints(), WidthFallback::Floor, 30)
            .await
            .unwrap();
        assert_eq!((held.kept, held.unrouted), ("full", 0));
        assert_eq!(held.ses, fixture.path().join("held/full/full.ses"));
        assert_eq!(held.passes.len(), 3);
        assert_eq!(row(&held, 0), ("full", "kept", None));
        assert_eq!(row(&held, 1), ("fallback", "skipped", Some("nothing open")));
        assert_eq!(row(&held, 2), ("polish", "skipped", Some("nothing open")));
        assert!(!ran(&fixture, "fallback"));
        assert!(!ran(&fixture, "polish"));
    }

    #[tokio::test]
    async fn a_fallback_that_closes_some_is_kept_and_polished() {
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=1\nunrouted.polish=1\n");
        let held = hold(&fixture, &floor_hints(), WidthFallback::Floor, 60)
            .await
            .unwrap();
        assert_eq!((held.kept, held.unrouted), ("polish", 1));
        assert_eq!(held.ses, fixture.path().join("held/polish/polish.ses"));
        assert_eq!(row(&held, 0), ("full", "kept", None));
        assert_eq!(row(&held, 1), ("fallback", "kept", None));
        assert_eq!(row(&held, 2), ("polish", "kept", None));
        let counts: Vec<_> = held.passes.iter().map(|r| r["unrouted"].as_u64()).collect();
        assert_eq!(counts, [Some(3), Some(1), Some(1)]);
        // The held time is the sum of the passes that ran.
        let sum: f64 = held
            .passes
            .iter()
            .map(|r| r["seconds"].as_f64().unwrap())
            .sum();
        assert!(
            (held.seconds - sum).abs() < 1e-9,
            "{} vs {sum}",
            held.seconds
        );
        assert!(held.seconds > 0.0);
    }

    #[tokio::test]
    async fn a_polish_that_leaves_more_open_is_discarded() {
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=1\nunrouted.polish=2\n");
        let held = hold(&fixture, &floor_hints(), WidthFallback::Floor, 60)
            .await
            .unwrap();
        assert_eq!((held.kept, held.unrouted), ("fallback", 1));
        assert_eq!(held.ses, fixture.path().join("held/fallback/fallback.ses"));
        assert_eq!(row(&held, 2).1, "discarded");
        assert!(row(&held, 2).2.is_some());
        assert_eq!(held.passes[2]["unrouted"].as_u64(), Some(2));
    }

    #[tokio::test]
    async fn a_fallback_that_closes_none_is_discarded_and_polish_is_not_run() {
        for fallback_unrouted in [3, 4] {
            let fixture = Fixture::new(&format!(
                "unrouted.full=3\nunrouted.fallback={fallback_unrouted}\nunrouted.polish=0\n"
            ));
            let held = hold(&fixture, &floor_hints(), WidthFallback::Floor, 60)
                .await
                .unwrap();
            assert_eq!(
                (held.kept, held.unrouted),
                ("full", 3),
                "{fallback_unrouted}"
            );
            assert_eq!(held.ses, fixture.path().join("held/full/full.ses"));
            assert_eq!(row(&held, 1).1, "discarded");
            assert!(row(&held, 1).2.is_some());
            assert_eq!(row(&held, 2).1, "skipped");
            assert!(
                !ran(&fixture, "polish"),
                "polish ran after a discarded Fallback"
            );
        }
    }

    #[tokio::test]
    async fn no_width_fallback_skips_the_fallback_with_its_reason() {
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=0\n");
        let held = hold(&fixture, &floor_hints(), WidthFallback::None, 30)
            .await
            .unwrap();
        assert_eq!((held.kept, held.unrouted), ("full", 3));
        assert_eq!(row(&held, 1).1, "skipped");
        assert!(row(&held, 1).2.unwrap().contains("fallback is off"));
        assert_eq!(row(&held, 2).1, "skipped");
        assert!(!ran(&fixture, "fallback"));
        assert!(!ran(&fixture, "polish"));
    }

    #[tokio::test]
    async fn no_floor_skips_the_fallback_with_the_manifests_reason() {
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=0\n");
        let no_floor = RoutingHints {
            copper_edge_clearance_um: None,
            retry_width_um: None,
            retry_off_reason: Some(RETRY_OFF_DIFFERENT_FLOORS_TEXT),
        };
        let held = hold(&fixture, &no_floor, WidthFallback::Floor, 30)
            .await
            .unwrap();
        assert_eq!(held.kept, "full");
        assert_eq!(
            row(&held, 1),
            ("fallback", "skipped", Some(RETRY_OFF_DIFFERENT_FLOORS_TEXT))
        );
        assert!(!ran(&fixture, "fallback"));
        assert!(!ran(&fixture, "polish"));
    }

    const RETRY_OFF_DIFFERENT_FLOORS_TEXT: &str = "net classes have different floors";

    #[tokio::test]
    async fn the_passes_get_their_own_retry_optimizer_and_start_session() {
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=1\nunrouted.polish=1\n");
        let held = hold(&fixture, &floor_hints(), WidthFallback::Floor, 60)
            .await
            .unwrap();
        let folder = fixture.path().join("held");
        let full = pass_args(&fixture, "full");
        let fallback = pass_args(&fixture, "fallback");
        let polish = pass_args(&fixture, "polish");

        assert_eq!(value(&full, "--router.neck_width_um="), Some("0"));
        assert_eq!(value(&full, "--router.optimizer.enabled="), Some("true"));
        assert!(value(&full, "--initial-session=").is_none());

        assert_eq!(value(&fallback, "--router.neck_width_um="), Some("120"));
        assert_eq!(
            value(&fallback, "--router.optimizer.enabled="),
            Some("false")
        );
        assert_eq!(
            value(&fallback, "--initial-session="),
            Some(folder.join("full/full.ses").to_str().unwrap())
        );

        assert_eq!(value(&polish, "--router.neck_width_um="), Some("0"));
        assert_eq!(value(&polish, "--router.optimizer.enabled="), Some("true"));
        assert_eq!(
            value(&polish, "--initial-session="),
            Some(folder.join("fallback/fallback.ses").to_str().unwrap())
        );
        for args in [&full, &fallback, &polish] {
            assert_eq!(
                value(args, "--router.copper_to_edge_clearance_um="),
                Some("250.001")
            );
        }
        assert_eq!(held.kept, "polish");
    }

    /// The pairs file is one input of the whole call: every pass of the hold
    /// sequence (Full, Fallback and Polish) gets the same `--pairs` token, and the
    /// sandboxed run can read the file, which lives outside every run folder.
    #[tokio::test]
    async fn every_pass_of_the_hold_gets_the_pairs_file_and_can_read_it() {
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=1\nunrouted.polish=1\n");
        let pairs = fixture.path().join("pairs.txt");
        let text = "pair /CAN_H /CAN_L gap=0.2\n";
        std::fs::write(&pairs, text).unwrap();
        let held = route_held(
            &fixture.binary(),
            &fixture.path().join("held"),
            &fixture.dsn(),
            &floor_hints(),
            CallFiles {
                amps: None,
                pairs: Some(&pairs),
            },
            WidthFallback::Floor,
            soon(60),
        )
        .await
        .unwrap();
        assert_eq!(held.kept, "polish", "all three passes ran");
        let expected = format!("--pairs={}", pairs.display());
        for name in ["full", "fallback", "polish"] {
            let args = pass_args(&fixture, name);
            assert_eq!(
                args.iter().filter(|token| **token == expected).count(),
                1,
                "{name}: {args:?}"
            );
            let read = std::fs::read_to_string(
                fixture
                    .path()
                    .join("held")
                    .join(name)
                    .join(format!("{name}.ses.pairs")),
            )
            .unwrap();
            assert_eq!(read, text, "{name}: what the sandboxed run read");
        }
    }

    /// The DSN of the runner tests: one plane, two components and two nets, in the
    /// shape Konnect exports.
    const RUNNER_DSN: &str = "(pcb board.kicad_pcb\n  (parser\n    (string_quote \")\n    \
(space_in_quoted_tokens on)\n    (host_cad Konnect)\n  )\n  (resolution um 10)\n  (unit um)\n  \
(structure\n    (layer F.Cu\n      (type signal)\n    )\n    (boundary\n      \
(path pcb 0 0 0 100 0 100 100 0 100)\n    )\n    (plane GND\n      \
(polygon F.Cu 0 1 1 9 1 9 9 1 9)\n    )\n    (via konnect_via_0001)\n    (rule\n      \
(width 200)\n      (clearance 200)\n    )\n  )\n  (placement\n    (component img_C1\n      \
(place C1 1000 -1000 front 90)\n    )\n    (component img_C2\n      \
(place C2 2000 -1000 front 90)\n    )\n  )\n  (library\n    (image img_C1\n      \
(pin pad_1 1 0 0)\n    )\n  )\n  (network\n    (net /CAN_H\n      (pins C1-1 C2-1)\n    )\n    \
(net /CAN_L\n      (pins C1-2 C2-2)\n    )\n  )\n  (wiring\n  )\n)\n";

    /// Every run of a search, whatever its shape and seed, routes with the call's
    /// pairs file on every pass: the production runner hands it to each run.
    #[tokio::test]
    async fn every_run_of_a_search_gets_the_pairs_switch() {
        use crate::route_judge::JudgeContext;
        use crate::route_search::{Runner, Shape};
        use crate::router::ToolRouter;
        use crate::tools::route_fastroute::FastrouteRunner;
        use crate::tools::{ServerConfig, ToolContext};
        use std::sync::Arc;

        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=1\nunrouted.polish=1\n");
        let search = fixture.path().join("search");
        std::fs::create_dir_all(&search).unwrap();
        let pairs = search.join("pairs.txt");
        std::fs::write(&pairs, "pair /CAN_H /CAN_L\n").unwrap();
        let runner = |pairs: Option<PathBuf>| FastrouteRunner {
            binary: fixture.binary(),
            dsn_text: Arc::new(RUNNER_DSN.to_string()),
            hints: floor_hints(),
            amps: None,
            pairs,
            fallback: WidthFallback::Floor,
            folder: search.clone(),
            judge: Arc::new(JudgeContext {
                board: fixture.path().join("board.kicad_pcb"),
                board_text: String::new(),
                manifest: json!({}),
                manifest_text: "{}".to_string(),
                kicad_cli: String::new(),
                pour_nets: Default::default(),
                power_nets: Default::default(),
                floors_mm: Default::default(),
                root: search.join("judge"),
                kicad_config_home: None,
            }),
            tools: Arc::new(ToolContext::new(
                ServerConfig::default(),
                Arc::new(ToolRouter::new()),
            )),
        };
        let runs = [
            (0, Shape::AsExported, 1),
            (1, Shape::Omitted, 1),
            (2, Shape::AsExported, 2),
            (3, Shape::Omitted, 2),
        ];
        let with = runner(Some(pairs.clone()));
        for (index, shape, seed) in runs {
            let routed = with.route(index, shape, seed, soon(60)).await.unwrap();
            assert_eq!(
                routed.held.kept, "polish",
                "run {index}: all three passes ran"
            );
            for name in ["full", "fallback", "polish"] {
                let args = std::fs::read_to_string(
                    routed.folder.join(name).join(format!("{name}.ses.args")),
                )
                .unwrap();
                let hits = args
                    .lines()
                    .filter(|line| *line == format!("--pairs={}", pairs.display()))
                    .count();
                assert_eq!(hits, 1, "run {index} pass {name}: {args}");
            }
        }
        // Control: a runner with no pairs file sends the switch on no run.
        let without = runner(None);
        let routed = without
            .route(10, Shape::AsExported, 1, soon(60))
            .await
            .unwrap();
        for name in ["full", "fallback", "polish"] {
            let args =
                std::fs::read_to_string(routed.folder.join(name).join(format!("{name}.ses.args")))
                    .unwrap();
            assert!(!args.contains("--pairs"), "{name}: {args}");
        }
    }

    /// Control: a call without a pairs file sends no `--pairs` on any pass.
    #[tokio::test]
    async fn a_call_without_pairs_sends_no_pairs_switch_on_any_pass() {
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=1\nunrouted.polish=1\n");
        let held = hold(&fixture, &floor_hints(), WidthFallback::Floor, 60)
            .await
            .unwrap();
        assert_eq!(held.kept, "polish");
        for name in ["full", "fallback", "polish"] {
            let args = pass_args(&fixture, name);
            assert!(
                !args.iter().any(|token| token.starts_with("--pairs")),
                "{name}: {args:?}"
            );
            assert!(!fixture
                .path()
                .join("held")
                .join(name)
                .join(format!("{name}.ses.pairs"))
                .exists());
        }
    }

    #[tokio::test]
    async fn a_pass_killed_by_the_deadline_ends_the_run_even_after_full_succeeded() {
        let fixture = Fixture::new("unrouted.full=3\nsleep.fallback=1\n");
        let result = hold(&fixture, &floor_hints(), WidthFallback::Floor, 4).await;
        assert!(
            matches!(result, Err(PassError::Killed { .. })),
            "{result:?}"
        );
        // Full had finished and its session is on disk, but the run is no candidate.
        assert!(fixture.path().join("held/full/full.ses").is_file());

        // The same fixture without the sleep keeps going: a pass that was not cut
        // short is a candidate.
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=1\nunrouted.polish=1\n");
        assert!(hold(&fixture, &floor_hints(), WidthFallback::Floor, 60)
            .await
            .is_ok());

        // A Full killed before it finishes is Killed too.
        let fixture = Fixture::new("sleep.full=1\n");
        let result = hold(&fixture, &floor_hints(), WidthFallback::Floor, 2).await;
        assert!(
            matches!(result, Err(PassError::Killed { .. })),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn a_failed_full_fails_the_run_and_a_failed_later_pass_keeps_the_earlier_session() {
        // Full failing leaves nothing to keep.
        let fixture = Fixture::new("fail.full=1\n");
        let text = failed_text(hold(&fixture, &floor_hints(), WidthFallback::Floor, 30).await);
        assert!(text.contains("fake failure marker for full"), "{text}");

        // Fallback failing keeps Full; Polish is not run.
        let fixture = Fixture::new("unrouted.full=3\nfail.fallback=1\n");
        let held = hold(&fixture, &floor_hints(), WidthFallback::Floor, 30)
            .await
            .unwrap();
        assert_eq!((held.kept, held.unrouted), ("full", 3));
        assert_eq!(held.ses, fixture.path().join("held/full/full.ses"));
        assert_eq!(row(&held, 0), ("full", "kept", None));
        assert_eq!(row(&held, 1).0, "fallback");
        assert_eq!(row(&held, 1).1, "failed");
        assert!(
            row(&held, 1)
                .2
                .unwrap()
                .contains("fake failure marker for fallback"),
            "{:?}",
            held.passes[1]
        );
        assert!(held.passes[1]["unrouted"].is_null());
        assert_eq!(
            row(&held, 2),
            ("polish", "skipped", Some("no Fallback was kept"))
        );
        assert!(!ran(&fixture, "polish"));

        // Polish failing keeps Fallback.
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=1\nfail.polish=1\n");
        let held = hold(&fixture, &floor_hints(), WidthFallback::Floor, 30)
            .await
            .unwrap();
        assert_eq!((held.kept, held.unrouted), ("fallback", 1));
        assert_eq!(held.ses, fixture.path().join("held/fallback/fallback.ses"));
        assert_eq!(row(&held, 1), ("fallback", "kept", None));
        assert_eq!(row(&held, 2).1, "failed");
        assert!(row(&held, 2)
            .2
            .unwrap()
            .contains("fake failure marker for polish"));
        // The held time counts the passes that gave a time: Full and Fallback.
        let sum: f64 = held.passes[..2]
            .iter()
            .map(|r| r["seconds"].as_f64().unwrap())
            .sum();
        assert!((held.seconds - sum).abs() < 1e-9);

        // A failure reason is cut to 300 characters.
        let fixture = Fixture::new("unrouted.full=3\nfail.fallback=1\n");
        let held = hold(&fixture, &floor_hints(), WidthFallback::Floor, 30)
            .await
            .unwrap();
        assert!(held.passes[1]["reason"].as_str().unwrap().chars().count() <= 300);
    }

    #[tokio::test]
    async fn a_killed_later_pass_still_ends_the_run_with_killed() {
        let fixture = Fixture::new("unrouted.full=3\nunrouted.fallback=1\nsleep.polish=1\n");
        let result = hold(&fixture, &floor_hints(), WidthFallback::Floor, 4).await;
        assert!(
            matches!(result, Err(PassError::Killed { .. })),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn the_sandbox_works_under_a_hebrew_folder_name() {
        // The owner's boards live under an iCloud folder with a Hebrew name, so the
        // run folder can hold non-ASCII characters, and the profile's subpath must
        // still match it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let fixture = Fixture::new(&format!("escape=1\nport={port}\nunrouted=1\n"));
        let parent = fixture.path().join("\u{5ea}\u{5d5}\u{5d0}\u{5e8}");
        let folder = parent.join("run");
        std::fs::create_dir_all(&parent).unwrap();
        let outcome = run_pass(
            &fixture.binary(),
            &folder,
            &fixture.dsn(),
            &hints(Some(250), None),
            CallFiles::default(),
            pass("full", 0.0, true, None),
            soon(30),
        )
        .await
        .unwrap();
        assert_eq!(outcome.unrouted, 1);
        assert_eq!(outcome.ses, folder.join("full.ses"));
        assert!(std::fs::metadata(&outcome.ses).unwrap().len() > 0);
        assert!(folder.join("full.json").is_file());
        assert_eq!(
            escape_results(&folder),
            "write_outside=denied\nnetwork=denied\nexec=denied\n"
        );
        assert!(!parent.join("outside.txt").exists());
        // The profile names the real path, Hebrew name included.
        let profile = std::fs::read_to_string(folder.join("profile.sb")).unwrap();
        assert!(
            profile.contains("\u{5ea}\u{5d5}\u{5d0}\u{5e8}"),
            "{profile}"
        );
        drop(listener);
    }
}
