//! Tests of the fastroute module. No real fastroute is needed: where a binary is
//! needed, a small C program (`FAKE_C`, compiled once with the system `cc`) stands
//! in for it. A compiled program is used rather than a shell script because the
//! sandbox profile (decision 13) allows exactly one program to start: a `#!/bin/sh`
//! script would need `/bin/sh` and then `/bin/bash` (sh dispatches to it) to start
//! as well, and the profile refuses both. The fake reads `control.txt` from its own
//! folder, so one build serves every test.

use super::*;
use serde_json::json;
use std::collections::{BTreeSet, HashSet};

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
    args_with(
        pass,
        hints,
        CallFiles {
            amps,
            pairs: None,
            layer_heights: None,
        },
    )
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
                CallFiles {
                    amps,
                    pairs,
                    layer_heights: None,
                },
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
            layer_heights: None,
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
            layer_heights: None,
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

// ---- Row LT: layer heights and the Tune pass's command line -----------------

fn tune_inputs<'a>(
    file: &'a Path,
    classes: &'a [String],
    nets: &'a BTreeSet<String>,
    layer_heights: Option<&'a str>,
) -> TuneInputs<'a> {
    TuneInputs {
        tune: file,
        classes,
        nets,
        layer_heights,
    }
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| name.to_string()).collect()
}

fn class_list(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

fn tune_args(
    hints: &RoutingHints,
    files: CallFiles<'_>,
    classes: &[String],
    layer_heights: Option<&str>,
) -> Vec<String> {
    let nets = names(&["A"]);
    tune_pass_arguments(
        Path::new("/d/in.dsn"),
        Path::new("/d/out.ses"),
        Path::new("/d/out.json"),
        hints,
        files,
        &tune_inputs(Path::new("/d/tune.txt"), classes, &nets, layer_heights),
        Path::new("/d/with space/held.ses"),
    )
}

#[test]
fn the_layer_heights_switch_is_one_token_after_pairs_and_only_when_given() {
    assert_eq!(
        layer_heights_argument("0,0.55,1.58"),
        "--layer-heights=0,0.55,1.58"
    );
    for pass in [
        pass("full", 0.0, true, None),
        pass("fallback", 120.0, false, Some("/d/full.ses")),
        pass("polish", 0.0, true, Some("/d/fallback.ses")),
    ] {
        let with = args_with(
            &pass,
            &hints(Some(250), Some(120)),
            CallFiles {
                amps: Some(Path::new("/d/amps.txt")),
                pairs: Some(Path::new("/d/pairs.txt")),
                layer_heights: Some("0,1.58"),
            },
        );
        let at = |prefix: &str| with.iter().position(|token| token.starts_with(prefix));
        assert_eq!(value(&with, "--layer-heights="), Some("0,1.58"));
        assert_eq!(
            at("--layer-heights="),
            at("--pairs=").map(|pairs| pairs + 1),
            "{}: right after --pairs: {with:?}",
            pass.name
        );
        assert!(
            !with.iter().any(|token| token.starts_with("--tune")),
            "{}: routing passes never get --tune",
            pass.name
        );
        // Without a pair it follows the amps file, and the initial session stays last.
        let heights_only = args_with(
            &pass,
            &hints(None, None),
            CallFiles {
                amps: Some(Path::new("/d/amps.txt")),
                pairs: None,
                layer_heights: Some("0,1.58"),
            },
        );
        let heights_at = heights_only
            .iter()
            .position(|token| token.starts_with("--layer-heights="))
            .unwrap();
        assert!(heights_only[heights_at - 1].starts_with("--x.par_via_amps="));
        // Not given: not sent, and the line is the one the call had before Row LT.
        let without = args_with(&pass, &hints(Some(250), Some(120)), CallFiles::default());
        assert!(!without
            .iter()
            .any(|token| token.starts_with("--layer-heights")));
    }
}

#[test]
fn tune_pass_arguments_follow_the_documented_order() {
    let args = tune_args(
        &hints(Some(500), Some(120)),
        CallFiles {
            amps: Some(Path::new("/d/amps.txt")),
            pairs: None,
            layer_heights: Some("0,1.58"),
        },
        &class_list(&["konnect_class_0001", "power"]),
        Some("0,1.58"),
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
            "--router.optimizer.enabled=false",
            "--router.autorouter.ignore_net_classes=konnect_class_0001,power",
            "--router.copper_to_edge_clearance_um=500",
            "--router.autorouter.max_threads=2",
            "--router.optimizer.max_threads=2",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    expected.extend(CANDIDATE_SWITCHES.iter().map(|s| s.to_string()));
    expected.extend(
        [
            "--x.par_via_amps=/d/amps.txt",
            "--tune=/d/tune.txt",
            "--layer-heights=0,1.58",
            "--initial-session=/d/with space/held.ses",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    assert_eq!(args, expected);
}

#[test]
fn tune_pass_arguments_leave_out_what_is_not_given_and_never_send_a_forbidden_setting() {
    let amps = Path::new("/d/amps.txt");
    for (classes, hint, amps, heights) in [
        (class_list(&[]), hints(None, None), None, None),
        (
            class_list(&["a"]),
            hints(Some(250), Some(120)),
            Some(amps),
            Some("0,1.58"),
        ),
        (
            class_list(&["a", "b", "c"]),
            hints(Some(300), None),
            None,
            Some("0,1.58"),
        ),
    ] {
        // The call's pairs file is on `files`, as it is for the routing passes: the
        // Tune pass still never sends it.
        let args = tune_args(
            &hint,
            CallFiles {
                amps,
                pairs: Some(Path::new("/d/pairs.txt")),
                layer_heights: heights,
            },
            &classes,
            heights,
        );
        let mut seen = HashSet::new();
        for token in &args {
            assert!(
                seen.insert(key(token).to_string()),
                "{token} repeats a setting: {args:?}"
            );
            for forbidden in [
                "--pairs",
                "--live",
                "--multi-start",
                "--max-time",
                "--diagnose",
                "--router.neck_width_um",
            ] {
                assert!(!token.starts_with(forbidden), "forbidden {token}");
            }
        }
        for switch in CANDIDATE_SWITCHES.iter().chain(PRESET.iter()) {
            assert_eq!(
                args.iter()
                    .filter(|token| token.as_str() == *switch)
                    .count(),
                1,
                "{switch}"
            );
        }
        assert_eq!(
            value(&args, "--router.optimizer.enabled="),
            Some("false"),
            "the optimizer is off"
        );
        assert_eq!(
            value(&args, "--router.autorouter.ignore_net_classes="),
            (!classes.is_empty()).then(|| classes.join(",")).as_deref(),
            "every class is ignored, and the switch is left out when there is none"
        );
        assert_eq!(
            value(&args, "--router.copper_to_edge_clearance_um=").is_some(),
            hint.copper_edge_clearance_um.is_some()
        );
        assert_eq!(value(&args, "--x.par_via_amps=").is_some(), amps.is_some());
        assert_eq!(value(&args, "--layer-heights="), heights);
        assert_eq!(value(&args, "--tune="), Some("/d/tune.txt"));
        assert_eq!(
            args.last().unwrap(),
            "--initial-session=/d/with space/held.ses"
        );
    }
    // The 250 trap holds here too (Freerouting reads exactly 250 as "unset").
    let args = tune_args(
        &hints(Some(250), None),
        CallFiles::default(),
        &class_list(&["a"]),
        None,
    );
    assert_eq!(
        value(&args, "--router.copper_to_edge_clearance_um="),
        Some("250.001")
    );
}

/// The Tune line and the Polish line (the routing pass nearest to it: the optimizer
/// on, no neck) differ in exactly the Tune pass's settings and nothing else.
#[test]
fn the_polish_line_differs_from_the_tune_line_in_exactly_the_tune_settings() {
    let files = CallFiles {
        amps: Some(Path::new("/d/amps.txt")),
        pairs: Some(Path::new("/d/pairs.txt")),
        layer_heights: Some("0,1.58"),
    };
    let hint = hints(Some(500), Some(120));
    let polish = pass_arguments(
        Path::new("/d/in.dsn"),
        Path::new("/d/out.ses"),
        Path::new("/d/out.json"),
        &hint,
        files,
        &pass("polish", 0.0, true, Some("/d/with space/held.ses")),
    );
    let tune = tune_args(
        &hint,
        files,
        &class_list(&["konnect_class_0001"]),
        Some("0,1.58"),
    );
    let keys = |args: &[String]| -> BTreeSet<String> {
        args.iter().map(|token| key(token).to_string()).collect()
    };
    let (polish_keys, tune_keys) = (keys(&polish), keys(&tune));
    let only_polish: BTreeSet<_> = polish_keys.difference(&tune_keys).cloned().collect();
    let only_tune: BTreeSet<_> = tune_keys.difference(&polish_keys).cloned().collect();
    assert_eq!(
        only_polish,
        names(&["--pairs", "--router.neck_width_um"]),
        "what the Tune pass does not send"
    );
    assert_eq!(
        only_tune,
        names(&["--router.autorouter.ignore_net_classes", "--tune"]),
        "what only the Tune pass sends"
    );
    // The one setting both send with another value.
    let shared: Vec<&String> = polish_keys.intersection(&tune_keys).collect();
    let different: Vec<&&String> = shared
        .iter()
        .filter(|name| value(&polish, &format!("{name}=")) != value(&tune, &format!("{name}=")))
        .collect();
    assert_eq!(
        different
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>(),
        ["--router.optimizer.enabled"],
        "the optimizer: on for Polish, off for the Tune pass"
    );
    // Everything else is the same, in the same order.
    let common = |args: &[String]| -> Vec<String> {
        args.iter()
            .filter(|token| {
                let key = key(token);
                polish_keys.contains(key) && tune_keys.contains(key)
            })
            .filter(|token| !token.starts_with("--router.optimizer.enabled="))
            .cloned()
            .collect()
    };
    assert_eq!(common(&polish), common(&tune));
}

// ---- Row LT: the geometry comparison of two sessions (L6) -------------------

/// Session units per micrometre in the test sessions (`resolution um 10`).
const UNITS: i64 = 10;

fn wire(layer: &str, width_um: i64, points: &[(i64, i64)]) -> String {
    let coordinates: Vec<String> = points
        .iter()
        .flat_map(|(x, y)| [(x * UNITS).to_string(), (y * UNITS).to_string()])
        .collect();
    format!(
        "(wire (path {layer} {} {}))",
        width_um * UNITS,
        coordinates.join(" ")
    )
}

fn via(padstack: &str, x: i64, y: i64) -> String {
    format!("(via \"{padstack}\" {} {})", x * UNITS, y * UNITS)
}

fn session_with(resolution: &str, nets: &[(&str, String)]) -> String {
    let nets: Vec<String> = nets
        .iter()
        .map(|(name, items)| format!("(net {name} {items})"))
        .collect();
    format!(
        "(session board (base_design board) (placement (resolution um 10)) (was_is) \
         (routes (resolution {resolution}) (parser (host_cad Konnect)) (library_out) \
         (network_out {})))",
        nets.join(" ")
    )
}

fn session(nets: &[(&str, String)]) -> String {
    session_with("um 10", nets)
}

fn changed(before: &str, after: &str, nets: &[&str]) -> Vec<String> {
    changed_outside(before, after, &names(nets)).unwrap()
}

/// A session of three nets, the shape most of these tests start from.
fn base() -> String {
    session(&[
        (
            "A",
            wire("F.Cu", 250, &[(0, 0), (10000, 0), (10000, 5000)]) + &via("v", 10000, 5000),
        ),
        ("B", wire("F.Cu", 250, &[(0, 1000), (20000, 1000)])),
        ("C", wire("B.Cu", 200, &[(0, 2000), (0, 8000)])),
    ])
}

#[test]
fn identical_sessions_and_group_net_changes_name_nothing() {
    assert!(changed(&base(), &base(), &[]).is_empty());
    // A group net's wire moved: nothing outside the group changed.
    let moved = session(&[
        (
            "A",
            wire("F.Cu", 250, &[(0, 0), (10000, 300), (10000, 5000)]) + &via("v", 10000, 5000),
        ),
        ("B", wire("F.Cu", 250, &[(0, 1000), (20000, 1000)])),
        ("C", wire("B.Cu", 200, &[(0, 2000), (0, 8000)])),
    ]);
    assert!(changed(&base(), &moved, &["A"]).is_empty());
    // The same move, with A outside the group, is named.
    assert_eq!(changed(&base(), &moved, &["B"]), ["A"]);
    // Net names are compared as they are written.
    assert!(changed(&base(), &moved, &["a"]) == ["A"]);
}

#[test]
fn another_nets_wire_or_via_moved_added_or_removed_is_named() {
    let b = wire("F.Cu", 250, &[(0, 1000), (20000, 1000)]);
    let c = wire("B.Cu", 200, &[(0, 2000), (0, 8000)]);
    let a = wire("F.Cu", 250, &[(0, 0), (10000, 0), (10000, 5000)]) + &via("v", 10000, 5000);
    let cases: Vec<(&str, String, Vec<&str>)> = vec![
        (
            "a wire's end moved by 2 um",
            session(&[
                ("A", a.clone()),
                ("B", wire("F.Cu", 250, &[(0, 1000), (20002, 1000)])),
                ("C", c.clone()),
            ]),
            vec!["B"],
        ),
        (
            "a via moved",
            session(&[
                (
                    "A",
                    wire("F.Cu", 250, &[(0, 0), (10000, 0), (10000, 5000)])
                        + &via("v", 10003, 5000),
                ),
                ("B", b.clone()),
                ("C", c.clone()),
            ]),
            vec!["A"],
        ),
        (
            "a via added",
            session(&[
                ("A", a.clone()),
                ("B", b.clone() + &via("v", 100, 1000)),
                ("C", c.clone()),
            ]),
            vec!["B"],
        ),
        (
            "a via removed",
            session(&[
                ("A", wire("F.Cu", 250, &[(0, 0), (10000, 0), (10000, 5000)])),
                ("B", b.clone()),
                ("C", c.clone()),
            ]),
            vec!["A"],
        ),
        (
            "a via's padstack changed",
            session(&[
                (
                    "A",
                    wire("F.Cu", 250, &[(0, 0), (10000, 0), (10000, 5000)])
                        + &via("w", 10000, 5000),
                ),
                ("B", b.clone()),
                ("C", c.clone()),
            ]),
            vec!["A"],
        ),
        (
            "a wire added",
            session(&[
                ("A", a.clone()),
                (
                    "B",
                    b.clone() + &wire("F.Cu", 250, &[(0, 1500), (500, 1500)]),
                ),
                ("C", c.clone()),
            ]),
            vec!["B"],
        ),
        (
            "a net's wire removed",
            session(&[("A", a.clone()), ("B", b.clone()), ("C", String::new())]),
            vec!["C"],
        ),
        (
            "a net gone",
            session(&[("A", a.clone()), ("B", b.clone())]),
            vec!["C"],
        ),
        (
            "a net appearing",
            session(&[
                ("A", a.clone()),
                ("B", b.clone()),
                ("C", c.clone()),
                ("D", wire("F.Cu", 250, &[(0, 0), (500, 0)])),
            ]),
            vec!["D"],
        ),
        (
            "a layer changed",
            session(&[
                ("A", a.clone()),
                ("B", b.clone()),
                ("C", wire("F.Cu", 200, &[(0, 2000), (0, 8000)])),
            ]),
            vec!["C"],
        ),
        (
            "a width changed",
            session(&[
                ("A", a.clone()),
                ("B", b.clone()),
                ("C", wire("B.Cu", 201, &[(0, 2000), (0, 8000)])),
            ]),
            vec!["C"],
        ),
    ];
    for (what, after, expected) in cases {
        assert_eq!(changed(&base(), &after, &[]), expected, "{what}");
        assert_eq!(changed(&after, &base(), &[]), expected, "{what}, reversed");
    }
    // Several nets are named in sorted order, and the group's are left out.
    let all = session(&[
        ("A", wire("F.Cu", 250, &[(0, 0), (1, 1)])),
        ("B", wire("F.Cu", 250, &[(0, 1000), (1, 1000)])),
        ("C", wire("B.Cu", 200, &[(0, 2000), (0, 9000)])),
    ]);
    assert_eq!(changed(&base(), &all, &[]), ["A", "B", "C"]);
    assert_eq!(changed(&base(), &all, &["B"]), ["A", "C"]);
}

#[test]
fn one_trace_split_into_collinear_pieces_compares_equal() {
    let whole = session(&[("A", wire("F.Cu", 250, &[(0, 0), (10000, 0), (10000, 5000)]))]);
    let split = |items: &[String]| session(&[("A", items.concat())]);
    // Split at a point, each half its own wire.
    assert!(changed(
        &whole,
        &split(&[
            wire("F.Cu", 250, &[(0, 0), (4000, 0)]),
            wire("F.Cu", 250, &[(4000, 0), (10000, 0), (10000, 5000)]),
        ]),
        &[]
    )
    .is_empty());
    // Pieces in another order and direction, and an overlap: the same copper.
    assert!(changed(
        &whole,
        &split(&[
            wire("F.Cu", 250, &[(10000, 5000), (10000, 0)]),
            wire("F.Cu", 250, &[(6000, 0), (0, 0)]),
            wire("F.Cu", 250, &[(3000, 0), (10000, 0)]),
        ]),
        &[]
    )
    .is_empty());
    // A path whose middle point lies on the line is the same as the path without it.
    assert!(changed(
        &whole,
        &split(&[wire(
            "F.Cu",
            250,
            &[(0, 0), (5000, 0), (10000, 0), (10000, 2500), (10000, 5000)]
        )]),
        &[]
    )
    .is_empty());
    // A gap between the pieces is not merged away.
    assert_eq!(
        changed(
            &whole,
            &split(&[
                wire("F.Cu", 250, &[(0, 0), (4000, 0)]),
                wire("F.Cu", 250, &[(4100, 0), (10000, 0), (10000, 5000)]),
            ]),
            &[]
        ),
        ["A"]
    );
    // Pieces of another width or layer are not merged into the trace.
    assert_eq!(
        changed(
            &whole,
            &split(&[
                wire("F.Cu", 250, &[(0, 0), (4000, 0)]),
                wire("F.Cu", 300, &[(4000, 0), (10000, 0)]),
                wire("F.Cu", 250, &[(10000, 0), (10000, 5000)]),
            ]),
            &[]
        ),
        ["A"]
    );
    // Parallel traces on neighbouring lines are not merged either.
    let two = session(&[(
        "A",
        wire("F.Cu", 250, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 250, &[(0, 1), (1000, 1)]),
    )]);
    let one = session(&[("A", wire("F.Cu", 250, &[(0, 0), (1000, 0)]))]);
    assert_eq!(changed(&two, &one, &[]), ["A"]);
}

#[test]
fn the_comparison_rounds_to_a_micrometre_and_uses_the_sessions_own_resolution() {
    let at = |x: f64| {
        // Whole units: 10 per micrometre.
        let end = (x * UNITS as f64).round() as i64;
        session(&[(
            "A",
            format!("(wire (path F.Cu 2500 0 0 {end} 0)) (via \"v\" 0 {end})"),
        )])
    };
    // 0.4 um is below the rounding step; 2 um is above it.
    assert!(changed(&at(10000.0), &at(10000.4), &[]).is_empty());
    assert!(changed(&at(10000.0), &at(9999.6), &[]).is_empty());
    assert_eq!(changed(&at(10000.0), &at(10002.0), &[]), ["A"]);
    assert_eq!(changed(&at(10000.0), &at(9998.0), &[]), ["A"]);
    // The same geometry in another resolution and unit compares equal.
    let in_mm = |resolution: &str, scale: i64| {
        session_with(
            resolution,
            &[(
                "A",
                format!(
                    "(wire (path F.Cu {} 0 0 {} {})) (via \"v\" 0 {})",
                    250 * scale / 1000,
                    10000 * scale / 1000,
                    -5000 * scale / 1000,
                    10000 * scale / 1000
                ),
            )],
        )
    };
    let um1 = in_mm("um 1", 1000);
    assert!(changed(&um1, &in_mm("um 10", 10_000), &[]).is_empty());
    assert!(changed(&um1, &in_mm("mm 1000", 1000), &[]).is_empty());
    assert_eq!(changed(&um1, &in_mm("mm 1000", 1001), &[]), ["A"]);
}

#[test]
fn keywords_in_any_letter_case_read_the_same() {
    let lower = base();
    let upper = lower
        .replace("(session", "(SESSION")
        .replace("(routes", "(Routes")
        .replace("(resolution um 10) (parser", "(RESOLUTION UM 10) (parser")
        .replace("(network_out", "(NETWORK_OUT")
        .replace("(net ", "(NET ")
        .replace("(wire", "(WIRE")
        .replace("(path", "(PATH")
        .replace("(via", "(Via");
    assert_ne!(lower, upper);
    assert!(changed(&lower, &upper, &[]).is_empty());
    // And a change is seen through the other letter case.
    let moved = upper.replace("F.Cu 2500 0 10000", "F.Cu 2500 0 10020");
    assert_ne!(moved, upper);
    assert_eq!(changed(&lower, &moved, &[]), ["B"]);
}

#[test]
fn a_net_with_no_copper_equals_a_net_that_is_not_there() {
    let empty = session(&[("A", String::new())]);
    let none = session(&[]);
    assert!(changed(&empty, &none, &[]).is_empty());
    assert!(changed(&none, &empty, &[]).is_empty());
    // A net named in two entries is one net.
    let twice = session(&[
        ("A", wire("F.Cu", 250, &[(0, 0), (1000, 0)])),
        ("A", wire("F.Cu", 250, &[(1000, 0), (3000, 0)])),
    ]);
    let once = session(&[("A", wire("F.Cu", 250, &[(0, 0), (3000, 0)]))]);
    assert!(changed(&twice, &once, &[]).is_empty());
}

#[test]
fn a_session_that_cannot_be_read_is_an_error_naming_which_one() {
    let arc = session(&[("A", "(wire (qarc F.Cu 2500 0 0 10 0 0 10))".to_string())]);
    for (what, text) in [
        ("not a session", "(pcb board)".to_string()),
        ("not balanced", "(session b (routes".to_string()),
        ("a wire that is not a path", arc.clone()),
        (
            "an odd coordinate count",
            session(&[("A", "(wire (path F.Cu 2500 0 0 10))".to_string())]),
        ),
        (
            "an odd coordinate count with two points",
            session(&[("A", "(wire (path F.Cu 2500 0 0 10 0 5))".to_string())]),
        ),
        (
            "one point",
            session(&[("A", "(wire (path F.Cu 2500 0 0))".to_string())]),
        ),
        (
            "a coordinate that is not a number",
            session(&[("A", "(wire (path F.Cu 2500 0 0 x 0))".to_string())]),
        ),
        (
            "a coordinate too large",
            session(&[(
                "A",
                "(wire (path F.Cu 2500 0 0 99999999999999999 0))".to_string(),
            )]),
        ),
        (
            "a via without a position",
            session(&[("A", "(via \"v\" 5)".to_string())]),
        ),
        ("no routes", "(session b (base_design b))".to_string()),
    ] {
        assert!(copper_by_net(&text).is_err(), "{what}");
        let error = changed_outside(&text, &base(), &names(&[])).unwrap_err();
        assert!(format!("{error:#}").contains("before"), "{what}: {error:#}");
        let error = changed_outside(&base(), &text, &names(&[])).unwrap_err();
        assert!(format!("{error:#}").contains("after"), "{what}: {error:#}");
    }
    // A wire's descriptors are not shapes.
    let typed = session(&[(
        "A",
        "(wire (path F.Cu 2500 0 0 100 0) (type protect))".to_string(),
    )]);
    assert_eq!(copper_by_net(&typed).unwrap()["A"].pieces.len(), 1);
}

#[test]
fn copper_by_net_orders_the_ends_and_the_lists() {
    let session = session(&[(
        "A",
        wire("B.Cu", 250, &[(500, 0), (0, 0)])
            + &wire("F.Cu", 250, &[(0, 0), (0, 700)])
            + &via("z", 9, 9)
            + &via("a", 5, 5),
    )]);
    let copper = copper_by_net(&session).unwrap();
    assert_eq!(
        copper["A"],
        NetCopper {
            pieces: vec![
                ("B.Cu".to_string(), 250, [0, 0, 500, 0]),
                ("F.Cu".to_string(), 250, [0, 0, 0, 700]),
            ],
            vias: vec![("a".to_string(), [5, 5]), ("z".to_string(), [9, 9])],
        }
    );
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

// A switch for one pass (`<name>.<pass>=1`) or for every pass (`<name>=1`).
static int cfg_pass_flag(const char *name, const char *pass) {
    char k[300];
    snprintf(k, sizeof k, "%s.%s", name, pass);
    return cfg_int(k, cfg_int(name, 0));
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
    const char *dsn = NULL, *ses = NULL, *report = NULL, *pairs = NULL, *tune = NULL, *heights = NULL;
    int version = 0;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--version") == 0) version = 1;
        else if (strcmp(argv[i], "-de") == 0 && i + 1 < argc) dsn = argv[++i];
        else if (strcmp(argv[i], "-do") == 0 && i + 1 < argc) ses = argv[++i];
        else if (strncmp(argv[i], "--report=", 9) == 0) report = argv[i] + 9;
        else if (strncmp(argv[i], "--pairs=", 8) == 0) pairs = argv[i] + 8;
        else if (strncmp(argv[i], "--tune=", 7) == 0) tune = argv[i] + 7;
        else if (strncmp(argv[i], "--layer-heights=", 16) == 0) heights = argv[i] + 16;
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

    if (cfg_int("echo_pairs", 0)) {
        // Say what this pass was given, then fail: the failure reason keeps the
        // end of the log, so a call that fails every run still shows the switch.
        fprintf(stderr, "PAIRS_ARG=%s\n", pairs ? pairs : "none");
        fprintf(stderr, "TUNE_ARG=%s\n", tune ? tune : "none");
        fprintf(stderr, "HEIGHTS_ARG=%s\n", heights ? heights : "none");
        return 3;
    }

    if (tune && cfg_int("no_tune", 0)) {
        // A build without tuning: its own message, then failure.
        fprintf(stderr, "fake fastroute without tuning: unknown option --tune\n");
        return 2;
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
    if (report && !cfg_pass_flag("noreport", pass)) {
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
    if (!cfg_pass_flag("nosession", pass)) {
        // The session text: a copy of the file named by copy_session.<pass> (or
        // copy_session), else a stand-in.
        char src[4096];
        snprintf(key, sizeof key, "copy_session.%s", pass);
        int have = cfg(key, src, sizeof src) || cfg("copy_session", src, sizeof src);
        FILE *s = fopen(ses, "w");
        if (s) {
            if (have) {
                FILE *in = fopen(src, "r");
                if (in) {
                    char buf[4096];
                    size_t n;
                    while ((n = fread(buf, 1, sizeof buf, in)) > 0) fwrite(buf, 1, n, s);
                    fclose(in);
                } else {
                    fputs("UNREADABLE\n", s);
                }
            } else if (!cfg_int("empty_session", 0)) {
                fputs("(session fake)\n", s);
            }
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
        fn set_control(&self, control: &str) {
            std::fs::write(self.binary.parent().unwrap().join("control.txt"), control).unwrap();
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
                layer_heights: None,
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
                layer_heights: None,
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
            None,
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
                layer_heights: None,
            },
            WidthFallback::Floor,
            None,
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
            routing_heights: None,
            tune: None,
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
                lengths: None,
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

    /// One call of `route_specctra_dsn`'s fastroute engine through its real path
    /// (`route_located`, so `search_and_write` and `conclude`), with a fake fastroute
    /// that echoes its `--pairs` argument and fails, and a fake kicad-cli. The board
    /// is in `project/`, the DSN and its manifest in `exported/`; `project_intent`
    /// and `exported_intent` are the design intent files of the two folders.
    /// Returns the error result's body and the folder the failed call set aside.
    async fn failed_call(
        project_intent: Option<Value>,
        exported_intent: Option<Value>,
    ) -> (Value, PathBuf, tempfile::TempDir) {
        use crate::router::ToolRouter;
        use crate::tools::route_fastroute::{prepare_call, route_located};
        use crate::tools::{ServerConfig, ToolContext};
        use std::sync::Arc;

        let fixture = Fixture::new("echo_pairs=1\n");
        let root = fixture.path().to_path_buf();
        let project = root.join("project");
        let exported = root.join("exported");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&exported).unwrap();
        let board_text = "(kicad_pcb (version 20260206))\n";
        let board = project.join("b.kicad_pcb");
        std::fs::write(&board, board_text).unwrap();
        // /CAN_H sits in a class with a diff pair gap of 0.2 mm.
        std::fs::write(
            project.join("b.kicad_pro"),
            json!({ "net_settings": {
                "classes": [{ "name": "PAIR", "priority": 0, "diff_pair_gap": 0.2 }],
                "netclass_assignments": { "/CAN_H": ["PAIR"] } } })
            .to_string(),
        )
        .unwrap();
        for (folder, intent) in [(&project, project_intent), (&exported, exported_intent)] {
            if let Some(intent) = intent {
                std::fs::create_dir_all(folder.join(".konnect")).unwrap();
                std::fs::write(
                    folder.join(".konnect/design_intent.json"),
                    intent.to_string(),
                )
                .unwrap();
            }
        }
        let dsn = exported.join("board.dsn");
        std::fs::write(&dsn, RUNNER_DSN).unwrap();
        std::fs::write(
            exported.join("board.dsn.konnect.json"),
            json!({
                "schema_version": 1,
                "board_path": board.display().to_string(),
                "source_sha256": format!("{:x}", Sha256::digest(board_text.as_bytes())),
            })
            .to_string(),
        )
        .unwrap();
        let cli = root.join("kicad-cli");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ctx = Arc::new(ToolContext::new(
            ServerConfig {
                kicad_cli: cli.display().to_string(),
                ..ServerConfig::default()
            },
            Arc::new(ToolRouter::new()),
        ));
        let args = json!({
            "dsn_path": dsn,
            "ses_output_path": exported.join("out.ses"),
            "effort": "fast",
        });
        let call = prepare_call(&args).unwrap().expect("good arguments");
        let result = route_located(&args, ctx, call, Located::Found(fixture.binary()))
            .await
            .unwrap();
        assert!(result.is_error, "every fake pass fails");
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text");
        };
        let body: Value = serde_json::from_str(text).unwrap();
        // The folder the failed call set aside is where its files went.
        let kept = exported.join("out.search.failed-1");
        assert!(kept.is_dir(), "{body}");
        (body, kept, fixture.dir)
    }

    fn run_reasons(body: &Value) -> Vec<String> {
        body["search"]["runs"]
            .as_array()
            .unwrap_or_else(|| panic!("no runs: {body}"))
            .iter()
            .map(|run| run["reason"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn can_intent() -> Value {
        json!({ "schema_version": 1, "rails": [], "currents": [],
            "interfaces": [{ "name": "CAN", "kind": "can", "nets": ["CAN_H", "CAN_L"],
                "differential_pairs": [{ "positive": "CAN_H", "negative": "CAN_L" }] }] })
    }

    /// The real call path: the pair of the intent in the board's own folder is
    /// resolved against the DSN, its file is written into the call's folder, every
    /// run gets `--pairs=<that file>`, and the result reports the pair. A different
    /// intent beside the DSN is not the board's and is not read.
    #[tokio::test]
    async fn a_call_gives_every_run_the_boards_pairs_and_reports_them() {
        let decoy = json!({ "schema_version": 1, "rails": [], "currents": [],
            "interfaces": [{ "name": "OTHER", "kind": "x", "nets": ["DECOY_P", "DECOY_N"],
                "differential_pairs": [{ "positive": "DECOY_P", "negative": "DECOY_N" }] }] });
        let (body, kept, _keep) = failed_call(Some(can_intent()), Some(decoy)).await;
        // The file, in the call's own folder, with the gap of the positive net's class.
        assert_eq!(
            std::fs::read_to_string(kept.join("pairs.txt")).unwrap(),
            "pair /CAN_H /CAN_L gap=0.2\n"
        );
        // Every run was given it, by its path in the search folder (the folder was
        // set aside afterwards).
        let reasons = run_reasons(&body);
        assert_eq!(reasons.len(), 2, "{body}");
        let given = format!(
            "PAIRS_ARG={}",
            kept.to_string_lossy()
                .replace("out.search.failed-1", "out.search")
        );
        let given = format!("{given}/pairs.txt");
        for reason in &reasons {
            assert!(reason.contains(&given), "{reason} lacks {given}");
        }
        // The result.
        assert_eq!(
            body["differential_pairs"],
            json!([{ "positive": "CAN_H", "negative": "CAN_L", "dsn_positive": "/CAN_H",
                     "dsn_negative": "/CAN_L", "gap_mm": 0.2 }])
        );
        assert_eq!(body["differential_pairs_unresolved"], json!([]));
        assert!(body.get("differential_pairs_note").is_none());
    }

    /// Control: with no design intent in the board's folder, no pairs file is made,
    /// no run gets `--pairs`, and both result lists are empty. A call with an intent
    /// that names a net the DSN lacks reports the pair and sends nothing either.
    #[tokio::test]
    async fn a_call_without_pairs_sends_nothing_and_reports_empty_lists() {
        let (body, kept, _keep) = failed_call(None, None).await;
        assert!(!kept.join("pairs.txt").exists());
        let reasons = run_reasons(&body);
        assert_eq!(reasons.len(), 2, "{body}");
        for reason in &reasons {
            assert!(reason.contains("PAIRS_ARG=none"), "{reason}");
        }
        assert_eq!(body["differential_pairs"], json!([]));
        assert_eq!(body["differential_pairs_unresolved"], json!([]));

        let missing = json!({ "schema_version": 1, "rails": [], "currents": [],
            "interfaces": [{ "name": "X", "kind": "x", "nets": ["CAN_H", "NOT_THERE"],
                "differential_pairs": [{ "positive": "CAN_H", "negative": "NOT_THERE" }] }] });
        let (body, kept, _keep) = failed_call(Some(missing), None).await;
        assert!(!kept.join("pairs.txt").exists());
        for reason in run_reasons(&body) {
            assert!(reason.contains("PAIRS_ARG=none"), "{reason}");
        }
        assert_eq!(body["differential_pairs"], json!([]));
        assert_eq!(
            body["differential_pairs_unresolved"][0]["negative"],
            "NOT_THERE"
        );
    }

    // ─── Row LT: the length plan through the real call path ──────────────────

    const POURS_BOARD: &str = include_str!("../tests/fixtures/specctra_pours_routing.kicad_pcb");
    const POURS_PRO: &str = include_str!("../tests/fixtures/specctra_pours_routing.kicad_pro");
    const POURS_DSN: &str = include_str!("../tests/fixtures/specctra_pours_routing.dsn");
    const POURS_MANIFEST: &str =
        include_str!("../tests/fixtures/specctra_pours_routing.dsn.konnect.json");

    /// One call through `route_located` on the public pours fixture (six signals
    /// `S1` to `S6`, two copper layers) with a fake fastroute that echoes its
    /// switches and fails every pass, and a fake kicad-cli. The board, its project,
    /// its design intent and its rules file are in `project/`, the DSN and its
    /// manifest in `exported/`.
    struct PoursCall {
        result: crate::mcp::protocol::CallToolResult,
        body: Value,
        exported: PathBuf,
        _keep: tempfile::TempDir,
    }

    impl PoursCall {
        /// Where the call's search folder is, or where a failed call's files went.
        fn search(&self) -> PathBuf {
            self.exported.join("out.search")
        }
        fn kept(&self) -> PathBuf {
            self.exported.join("out.search.failed-1")
        }
        /// Nothing of the call is left: no search folder, none set aside, no output.
        fn assert_nothing_routed(&self) {
            assert!(self.result.is_error);
            assert!(!self.search().exists(), "a search folder was left");
            assert!(
                self.kept().symlink_metadata().is_err(),
                "a search folder was set aside: a run started"
            );
            assert!(!self.exported.join("out.ses").exists());
        }
        fn reasons(&self) -> Vec<String> {
            run_reasons(&self.body)
        }
    }

    /// `intent` is the design intent's text, `dru` the board's rules file; the edits
    /// change the DSN or the board before the call (the manifest is re-pinned to the
    /// edited board).
    async fn pours_call(
        intent: Option<&str>,
        dru: Option<&str>,
        dsn_edit: &dyn Fn(&str) -> String,
        board_edit: &dyn Fn(&str) -> String,
    ) -> PoursCall {
        use crate::router::ToolRouter;
        use crate::tools::route_fastroute::{prepare_call, route_located};
        use crate::tools::{ServerConfig, ToolContext};
        use std::sync::Arc;

        let fixture = Fixture::new("echo_pairs=1\n");
        let root = fixture.path().to_path_buf();
        let project = root.join("project");
        let exported = root.join("exported");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&exported).unwrap();
        let board_text = board_edit(POURS_BOARD);
        let board = project.join("b.kicad_pcb");
        std::fs::write(&board, &board_text).unwrap();
        std::fs::write(project.join("b.kicad_pro"), POURS_PRO).unwrap();
        if let Some(intent) = intent {
            std::fs::create_dir_all(project.join(".konnect")).unwrap();
            std::fs::write(project.join(".konnect/design_intent.json"), intent).unwrap();
        }
        if let Some(dru) = dru {
            std::fs::write(project.join("b.kicad_dru"), dru).unwrap();
        }
        let dsn = exported.join("board.dsn");
        std::fs::write(&dsn, dsn_edit(POURS_DSN)).unwrap();
        let mut manifest: Value = serde_json::from_str(POURS_MANIFEST).unwrap();
        manifest["board_path"] = json!(board.display().to_string());
        manifest["source_sha256"] = json!(format!("{:x}", Sha256::digest(board_text.as_bytes())));
        std::fs::write(
            exported.join("board.dsn.konnect.json"),
            manifest.to_string(),
        )
        .unwrap();
        let cli = root.join("kicad-cli");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ctx = Arc::new(ToolContext::new(
            ServerConfig {
                kicad_cli: cli.display().to_string(),
                ..ServerConfig::default()
            },
            Arc::new(ToolRouter::new()),
        ));
        let args = json!({
            "dsn_path": dsn,
            "ses_output_path": exported.join("out.ses"),
            "effort": "fast",
        });
        let call = prepare_call(&args).unwrap().expect("good arguments");
        let result = route_located(&args, ctx, call, Located::Found(fixture.binary()))
            .await
            .unwrap();
        let crate::mcp::protocol::ToolContent::Text { text } = &result.content[0] else {
            panic!("expected text");
        };
        let body: Value = serde_json::from_str(text).unwrap();
        PoursCall {
            result,
            body,
            exported,
            _keep: fixture.dir,
        }
    }

    fn unchanged(text: &str) -> String {
        text.to_string()
    }

    /// The fixture's interface `bus` (its six signals) with extra keys.
    fn bus_intent(extra: Value) -> String {
        let mut interface = json!({ "name": "bus", "kind": "bus",
            "nets": ["S1", "S2", "S3", "S4", "S5", "S6"] });
        for (key, value) in extra.as_object().unwrap() {
            interface[key] = value.clone();
        }
        json!({ "schema_version": 1, "rails": [], "currents": [], "interfaces": [interface] })
            .to_string()
    }

    fn group_intent() -> String {
        bus_intent(json!({ "length_groups": [
            { "name": "g", "nets": ["S1", "S2", "S3", "S4"], "tolerance_mm": 0.5 }] }))
    }

    /// Row LT, L4: a group or skew that cannot be applied refuses the call before
    /// anything is routed, and the search folder holds no run.
    #[tokio::test]
    async fn a_length_group_that_cannot_be_applied_refuses_before_anything_is_routed() {
        let rename = |from: &'static str, to: &'static str| move |dsn: &str| dsn.replace(from, to);
        let target = |target: f64| {
            bus_intent(json!({ "length_groups": [
                { "name": "g", "nets": ["S1", "S2"], "tolerance_mm": 0.5, "target_mm": target }] }))
        };
        let skew_on_a_lost_pair = bus_intent(json!({ "nets": ["S1", "S9"],
            "differential_pairs": [{ "positive": "S1", "negative": "S9", "max_skew_mm": 0.2 }] }));
        let unresolved = bus_intent(json!({ "nets": ["S1", "S9"], "length_groups": [
            { "name": "g", "nets": ["S1", "S9"], "tolerance_mm": 0.5 }] }));
        let unusable = json!({ "schema_version": 99, "rails": [], "currents": [], "interfaces": [
            { "name": "bus", "kind": "bus", "nets": ["S1", "S2"],
              "length_groups": [{ "name": "g", "nets": ["S1", "S2"], "tolerance_mm": 0.5 }] }] })
        .to_string();
        let cases: Vec<(
            &str,
            String,
            Box<dyn Fn(&str) -> String>,
            Box<dyn Fn(&str) -> String>,
            &str,
        )> = vec![
            (
                "an unresolved name",
                unresolved,
                Box::new(unchanged),
                Box::new(unchanged),
                "'S9'",
            ),
            (
                "an ambiguous name",
                group_intent(),
                Box::new(|dsn: &str| {
                    dsn.replace("(net S2", "(net /a/S2").replace(
                        "(network\n",
                        "(network\n    (net /b/S2\n      (pins R1-1)\n    )\n",
                    )
                }),
                Box::new(unchanged),
                "'S2'",
            ),
            (
                "a name the tune file cannot carry",
                group_intent(),
                Box::new(rename("(net S2", "(net /x*/S2")),
                Box::new(unchanged),
                "/x*/S2",
            ),
            (
                "a DSN net that is not a net of the board",
                group_intent(),
                Box::new(rename("(net S2", "(net /a/S2")),
                Box::new(unchanged),
                "/a/S2",
            ),
            (
                "a checked net equal to another board net ignoring case",
                group_intent(),
                Box::new(unchanged),
                Box::new(|board: &str| {
                    let end = board.rfind(')').unwrap();
                    format!(
                        "{}(zone (net \"s1\") (layer \"F.Cu\") (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10))))\n)\n",
                        &board[..end]
                    )
                }),
                "'S1'",
            ),
            (
                "a skew on a pair the pairs plan left out",
                skew_on_a_lost_pair,
                Box::new(unchanged),
                Box::new(unchanged),
                "'S9'",
            ),
            (
                "a target no net can meet",
                target(1.0),
                Box::new(unchanged),
                Box::new(unchanged),
                "cannot be that short",
            ),
            (
                "an unusable intent that declares lengths",
                unusable,
                Box::new(unchanged),
                Box::new(unchanged),
                "length_groups or max_skew_mm",
            ),
        ];
        for (what, intent, dsn_edit, board_edit, mentions) in cases {
            let call = pours_call(Some(&intent), None, &*dsn_edit, &*board_edit).await;
            call.assert_nothing_routed();
            assert_eq!(
                call.body["error"]["kind"], "invalid_configuration",
                "{what}: {}",
                call.body
            );
            let message = call.body["message"].as_str().unwrap_or_default();
            assert!(message.contains("Nothing was routed."), "{what}: {message}");
            assert!(message.contains(mentions), "{what}: {message}");
        }
    }

    /// The negative controls of the refusals above: the same board with names that
    /// resolve plans and routes (every fake pass fails, so the call ends with the
    /// runs it launched and the set-aside folder); an intent that cannot be used and
    /// does not declare lengths routes with D23's note, as before Row LT.
    #[tokio::test]
    async fn a_resolvable_group_routes_and_an_unusable_intent_without_lengths_routes_with_a_note() {
        let call = pours_call(Some(&group_intent()), None, &unchanged, &unchanged).await;
        assert!(call.result.is_error, "every fake pass fails");
        assert!(call.kept().is_dir(), "{}", call.body);
        assert_eq!(call.reasons().len(), 2, "{}", call.body);
        let tune = std::fs::read_to_string(call.kept().join("tune.txt")).unwrap();
        assert!(
            tune.contains("group lt1 tolerance=0.25\n  S1\n  S2\n  S3\n  S4\n"),
            "{tune}"
        );

        let unusable = json!({ "schema_version": 99, "rails": [] }).to_string();
        let call = pours_call(Some(&unusable), None, &unchanged, &unchanged).await;
        assert!(call.kept().is_dir(), "{}", call.body);
        assert_eq!(call.reasons().len(), 2, "{}", call.body);
        assert!(!call.kept().join("tune.txt").exists());
        let note = call.body["length_matching"]["note"]
            .as_str()
            .expect("a note");
        assert!(note.contains("cannot be used"), "{note}");
        assert!(
            call.body["differential_pairs_note"].is_string(),
            "{}",
            call.body
        );
    }

    /// L6 and L7: a call with a group writes `tune.txt` and reports the plan, but no
    /// routing pass gets `--tune`, and `--layer-heights` goes to the routing passes
    /// only with a resolved pair; a call with neither gets DP's command lines.
    #[tokio::test]
    async fn routing_passes_never_get_tune_and_get_heights_only_with_a_pair() {
        let grouped = pours_call(Some(&group_intent()), None, &unchanged, &unchanged).await;
        assert!(grouped.kept().join("tune.txt").is_file());
        assert_eq!(grouped.reasons().len(), 2);
        for reason in grouped.reasons() {
            assert!(reason.contains("TUNE_ARG=none"), "{reason}");
            assert!(reason.contains("HEIGHTS_ARG=none"), "{reason}");
        }
        let matching = &grouped.body["length_matching"];
        assert_eq!(matching["groups"][0]["file_name"], "lt1");
        assert_eq!(matching["layer_heights"]["sent"], true, "{matching}");
        assert_eq!(matching["groups"][0]["pass"], Value::Null);

        // A resolved pair: the routing passes get the heights, still no --tune.
        let paired = pours_call(
            Some(&bus_intent(json!({
                "differential_pairs": [{ "positive": "S1", "negative": "S2", "max_skew_mm": 0.2 }],
                "length_groups": [{ "name": "g", "nets": ["S1", "S2"], "tolerance_mm": 0.5 }] }))),
            None,
            &unchanged,
            &unchanged,
        )
        .await;
        assert_eq!(
            std::fs::read_to_string(paired.kept().join("pairs.txt")).unwrap(),
            "pair S1 S2 skew=0.1\n"
        );
        let heights = paired.body["length_matching"]["layer_heights"]["heights_mm"].clone();
        let wanted = format!(
            "HEIGHTS_ARG={}",
            heights
                .as_array()
                .expect("heights")
                .iter()
                .map(|height| height.as_f64().unwrap().to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        for reason in paired.reasons() {
            assert!(reason.contains(&wanted), "{reason} lacks {wanted}");
            assert!(reason.contains("TUNE_ARG=none"), "{reason}");
        }

        // Control: neither group nor pair, DP's exact lines.
        let plain = pours_call(None, None, &unchanged, &unchanged).await;
        for reason in plain.reasons() {
            assert!(
                reason.contains("TUNE_ARG=none") && reason.contains("HEIGHTS_ARG=none"),
                "{reason}"
            );
            assert!(reason.contains("PAIRS_ARG=none"), "{reason}");
        }
    }

    /// L18: a call without a group or a skew has no tune file and no
    /// `length_matching`, and a rules file without length rules adds none; the same
    /// board with one group gets the file and the field.
    #[tokio::test]
    async fn a_call_without_groups_has_no_tune_file_and_no_length_matching() {
        let plain_dru = "(version 1)\n(rule \"wide\" (constraint track_width (min 0.2mm)))\n";
        for (what, intent, dru) in [
            ("no intent", None, None),
            (
                "a pair without a skew",
                Some(bus_intent(json!({
                "differential_pairs": [{ "positive": "S1", "negative": "S2" }] }))),
                None,
            ),
            ("a rules file without length rules", None, Some(plain_dru)),
        ] {
            let call = pours_call(intent.as_deref(), dru, &unchanged, &unchanged).await;
            assert!(call.kept().is_dir(), "{what}: {}", call.body);
            assert!(!call.kept().join("tune.txt").exists(), "{what}");
            assert!(
                call.body.get("length_matching").is_none(),
                "{what}: {}",
                call.body
            );
            for reason in call.reasons() {
                assert!(reason.contains("TUNE_ARG=none"), "{what}: {reason}");
            }
        }
        // Control: one group gives the file and the field.
        let call = pours_call(Some(&group_intent()), None, &unchanged, &unchanged).await;
        assert!(call.kept().join("tune.txt").is_file());
        assert!(call.body["length_matching"].is_object(), "{}", call.body);
        // A KiCad length rule alone is reported (listed, not evaluated), still without
        // a tune file.
        let length_dru = "(version 1)\n(rule \"len\" (constraint length (max 60mm)) (condition \"A.NetName == 'S1'\"))\n";
        let call = pours_call(None, Some(length_dru), &unchanged, &unchanged).await;
        assert!(!call.kept().join("tune.txt").exists());
        let rules = &call.body["length_matching"]["kicad_rules"];
        assert_eq!(rules[0]["name"], "len", "{}", call.body);
        assert_eq!(rules[0]["evaluated"], false);
        assert_eq!(call.body["length_matching"]["groups"], json!([]));
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

    // ---- Row LT: the Tune pass ------------------------------------------------

    /// Runs the hold sequence with a Tune pass whose input session is `before`
    /// (Full's) and whose output is `after`, both given to the fake as files.
    /// `control` adds the fake's other settings. The call also gives the routing
    /// passes a pairs file and layer heights of their own, which the Tune pass must
    /// not take. Returns the fixture and the run's result.
    async fn tune_run(
        before: &str,
        after: &str,
        control: &str,
        nets: &[&str],
        seconds: u64,
    ) -> (Fixture, std::result::Result<Held, PassError>) {
        let fixture = Fixture::new("");
        let before_path = fixture.path().join("before.ses");
        let after_path = fixture.path().join("after.ses");
        std::fs::write(&before_path, before).unwrap();
        std::fs::write(&after_path, after).unwrap();
        fixture.set_control(&format!(
            "copy_session.full={}\ncopy_session.tune={}\n{control}",
            before_path.display(),
            after_path.display()
        ));
        let tune_file = fixture.path().join("tune.txt");
        std::fs::write(&tune_file, "group lt1 tolerance=0.5\n  A\n").unwrap();
        let pairs = fixture.path().join("pairs.txt");
        std::fs::write(&pairs, "pair /X /Y gap=0.2\n").unwrap();
        let classes = class_list(&["c1", "c2"]);
        let nets = names(nets);
        let result = route_held(
            &fixture.binary(),
            &fixture.path().join("held"),
            &fixture.dsn(),
            &floor_hints(),
            CallFiles {
                amps: None,
                pairs: Some(&pairs),
                layer_heights: Some("9,9"),
            },
            WidthFallback::None,
            Some(tune_inputs(&tune_file, &classes, &nets, Some("0,1.58"))),
            soon(seconds),
        )
        .await;
        (fixture, result)
    }

    /// Full, then Fallback and Polish skipped, then the Tune pass: its row.
    fn tune_row(held: &Held) -> (&str, &str, Option<&str>) {
        assert_eq!(held.passes.len(), 4, "{:?}", held.passes);
        row(held, 3)
    }

    /// `base()` with A changed (a meander) and B written as two collinear pieces: the
    /// copper of B is the same geometry.
    fn tuned_base() -> String {
        session(&[
            (
                "A",
                wire(
                    "F.Cu",
                    250,
                    &[
                        (0, 0),
                        (3000, 0),
                        (3000, 800),
                        (3500, 800),
                        (3500, 0),
                        (10000, 0),
                        (10000, 5000),
                    ],
                ) + &via("v", 10000, 5000),
            ),
            (
                "B",
                wire("F.Cu", 250, &[(20000, 1000), (9000, 1000)])
                    + &wire("F.Cu", 250, &[(0, 1000), (9000, 1000)]),
            ),
            ("C", wire("B.Cu", 200, &[(0, 2000), (0, 8000)])),
        ])
    }

    #[tokio::test]
    async fn a_tune_pass_that_changes_only_group_nets_is_kept() {
        let (fixture, held) = tune_run(&base(), &tuned_base(), "", &["A"], 30).await;
        let held = held.unwrap();
        assert_eq!(tune_row(&held), ("tune", "kept", None));
        assert_eq!(held.kept, TUNE_PASS);
        assert_eq!(held.ses, fixture.path().join("held/tune/tune.ses"));
        assert_eq!(held.unrouted, 0);
        // Its session is the one the run now holds.
        assert_eq!(std::fs::read_to_string(&held.ses).unwrap(), tuned_base());
        // The row joins the others, and the pass's time joins the run's.
        assert_eq!(row(&held, 0), ("full", "kept", None));
        assert_eq!(row(&held, 1).1, "skipped");
        assert_eq!(row(&held, 2).1, "skipped");
        let seconds: f64 = [0, 3]
            .iter()
            .map(|&i| held.passes[i]["seconds"].as_f64().unwrap())
            .sum();
        assert!((held.seconds - seconds).abs() < 1e-9, "{}", held.seconds);
        assert_eq!(held.passes[3]["unrouted"], 0);

        // What the pass was given: the held session, the tune file, every class
        // ignored, the optimizer off, its own heights, and no pairs file.
        let args = pass_args(&fixture, "tune");
        let full = fixture.path().join("held/full/full.ses");
        assert_eq!(
            value(&args, "--initial-session="),
            Some(full.to_str().unwrap())
        );
        assert_eq!(
            value(&args, "--tune="),
            Some(fixture.path().join("tune.txt").to_str().unwrap())
        );
        assert_eq!(
            value(&args, "--router.autorouter.ignore_net_classes="),
            Some("c1,c2")
        );
        assert_eq!(value(&args, "--router.optimizer.enabled="), Some("false"));
        assert_eq!(value(&args, "--layer-heights="), Some("0,1.58"));
        assert!(!args.iter().any(|t| t.starts_with("--pairs")), "{args:?}");
        assert!(!args.iter().any(|t| t.starts_with("--router.neck_width_um")));
        assert!(!fixture.path().join("held/tune/tune.ses.pairs").exists());
        assert_eq!(
            args.last().unwrap(),
            &format!("--initial-session={}", full.display())
        );
    }

    #[tokio::test]
    async fn a_tune_pass_that_changes_another_nets_copper_is_discarded_naming_it() {
        let moved_b = session(&[
            (
                "A",
                wire("F.Cu", 250, &[(0, 0), (10000, 0), (10000, 5000)]) + &via("v", 10000, 5000),
            ),
            ("B", wire("F.Cu", 250, &[(0, 1000), (20000, 1002)])),
            ("C", wire("B.Cu", 200, &[(0, 2000), (0, 8000)])),
        ]);
        let (fixture, held) = tune_run(&base(), &moved_b, "", &["A"], 30).await;
        let held = held.unwrap();
        assert_eq!(
            tune_row(&held),
            ("tune", "discarded", Some("changed the copper of B"))
        );
        // The run keeps its input session.
        assert_eq!(held.kept, "full");
        assert_eq!(held.ses, fixture.path().join("held/full/full.ses"));
        assert_eq!(held.passes[3]["unrouted"], 0);
        // A discarded pass's time still counts, as Fallback's does.
        let seconds: f64 = [0, 3]
            .iter()
            .map(|&i| held.passes[i]["seconds"].as_f64().unwrap())
            .sum();
        assert!((held.seconds - seconds).abs() < 1e-9);

        // A via moved, a net vanished and a net added are each named; the group's
        // own net is not.
        let after = session(&[
            (
                "A",
                wire("F.Cu", 250, &[(0, 0), (10000, 100)]) + &via("v", 10000, 5000),
            ),
            (
                "B",
                wire("F.Cu", 250, &[(0, 1000), (20000, 1000)]) + &via("v", 5, 5),
            ),
            ("D", wire("F.Cu", 250, &[(0, 0), (500, 0)])),
        ]);
        let (_fixture, held) = tune_run(&base(), &after, "", &["A"], 30).await;
        assert_eq!(
            tune_row(&held.unwrap()),
            ("tune", "discarded", Some("changed the copper of B, C, D"))
        );
        // The same pass with the group's net the only change is kept (the control).
        let only_a = session(&[
            ("A", wire("F.Cu", 250, &[(0, 0), (10000, 100)])),
            ("B", wire("F.Cu", 250, &[(0, 1000), (20000, 1000)])),
            ("C", wire("B.Cu", 200, &[(0, 2000), (0, 8000)])),
        ]);
        let (_fixture, held) = tune_run(&base(), &only_a, "", &["A"], 30).await;
        assert_eq!(tune_row(&held.unwrap()).1, "kept");
    }

    #[tokio::test]
    async fn a_discarded_tune_pass_names_at_most_five_nets() {
        let many = |count: usize, shift: i64| {
            let nets: Vec<(String, String)> = (1..=count)
                .map(|n| {
                    (
                        format!("N{n}"),
                        wire(
                            "F.Cu",
                            250,
                            &[(0, 100 * n as i64), (5000 + shift, 100 * n as i64)],
                        ),
                    )
                })
                .collect();
            let mut all = vec![("A".to_string(), wire("F.Cu", 250, &[(0, 0), (1000, 0)]))];
            all.extend(nets);
            let borrowed: Vec<(&str, String)> = all
                .iter()
                .map(|(name, items)| (name.as_str(), items.clone()))
                .collect();
            session(&borrowed)
        };
        for (count, expected) in [
            (1, "changed the copper of N1"),
            (5, "changed the copper of N1, N2, N3, N4, N5"),
            (6, "changed the copper of N1, N2, N3, N4, N5 and 1 more"),
            (9, "changed the copper of N1, N2, N3, N4, N5 and 4 more"),
        ] {
            let (_fixture, held) =
                tune_run(&many(count, 0), &many(count, 20), "", &["A"], 30).await;
            assert_eq!(
                tune_row(&held.unwrap()),
                ("tune", "discarded", Some(expected)),
                "{count} nets"
            );
        }
    }

    #[tokio::test]
    async fn a_tune_pass_that_leaves_more_connections_open_is_discarded() {
        // Full left 1 open; the Tune pass left 3: it opened 2.
        let (fixture, held) = tune_run(
            &base(),
            &tuned_base(),
            "unrouted.full=1\nunrouted.tune=3\n",
            &["A"],
            30,
        )
        .await;
        let held = held.unwrap();
        assert_eq!(tune_row(&held), ("tune", "discarded", Some("opened 2")));
        assert_eq!(held.passes[3]["unrouted"], 3);
        assert_eq!((held.kept, held.unrouted), ("full", 1));
        assert_eq!(held.ses, fixture.path().join("held/full/full.ses"));

        // One more is one: "opened 1".
        let (_fixture, held) = tune_run(
            &base(),
            &tuned_base(),
            "unrouted.full=1\nunrouted.tune=2\n",
            &["A"],
            30,
        )
        .await;
        assert_eq!(
            tune_row(&held.unwrap()),
            ("tune", "discarded", Some("opened 1"))
        );

        // Control: the same count, or fewer, is kept, and the run holds its count.
        for (tune_open, expected) in [(1, 1), (0, 0)] {
            let (_fixture, held) = tune_run(
                &base(),
                &tuned_base(),
                &format!("unrouted.full=1\nunrouted.tune={tune_open}\n"),
                &["A"],
                30,
            )
            .await;
            let held = held.unwrap();
            assert_eq!(tune_row(&held), ("tune", "kept", None));
            assert_eq!((held.kept, held.unrouted), (TUNE_PASS, expected));
        }
    }

    #[tokio::test]
    async fn a_failed_tune_pass_is_a_failed_row_and_the_run_keeps_its_input() {
        let (fixture, held) = tune_run(&base(), &tuned_base(), "fail.tune=1\n", &["A"], 30).await;
        let held = held.unwrap();
        let (name, outcome, reason) = tune_row(&held);
        assert_eq!((name, outcome), ("tune", "failed"));
        assert!(
            reason.unwrap().contains("fake failure marker for tune"),
            "{reason:?}"
        );
        assert!(held.passes[3]["unrouted"].is_null());
        assert_eq!((held.kept, held.unrouted), ("full", 0));
        assert_eq!(held.ses, fixture.path().join("held/full/full.ses"));
        // No session of the Tune pass is in play, and a failed pass adds no time.
        assert!((held.seconds - held.passes[0]["seconds"].as_f64().unwrap()).abs() < 1e-9);

        // A build that does not know `--tune` fails with its own message, and the
        // run still ends with its input. (The fake that accepts it is the kept test.)
        let (_fixture, held) = tune_run(&base(), &tuned_base(), "no_tune=1\n", &["A"], 30).await;
        let held = held.unwrap();
        let (name, outcome, reason) = tune_row(&held);
        assert_eq!((name, outcome), ("tune", "failed"));
        assert!(
            reason
                .unwrap()
                .contains("fake fastroute without tuning: unknown option --tune"),
            "{reason:?}"
        );
        assert_eq!(held.kept, "full");
        // A pass that leaves no session, or no report, fails too.
        for control in ["nosession.tune=1\n", "noreport.tune=1\n"] {
            let (_fixture, held) = tune_run(&base(), &tuned_base(), control, &["A"], 30).await;
            let held = held.unwrap();
            assert_eq!(tune_row(&held).1, "failed", "{control}");
            assert_eq!(held.kept, "full");
        }
    }

    #[tokio::test]
    async fn a_tune_pass_killed_by_the_deadline_ends_the_run() {
        let (fixture, result) = tune_run(&base(), &tuned_base(), "sleep.tune=1\n", &["A"], 4).await;
        assert!(
            matches!(result, Err(PassError::Killed { .. })),
            "{result:?}"
        );
        // Full had finished and kept its session, but a run cut short is no candidate.
        assert!(fixture.path().join("held/full/full.ses").is_file());
    }

    #[tokio::test]
    async fn sessions_that_cannot_be_compared_discard_the_tune_pass() {
        // The Tune pass wrote something that is no session.
        let (_fixture, held) = tune_run(&base(), "not a session", "", &["A"], 30).await;
        let held = held.unwrap();
        let (name, outcome, reason) = tune_row(&held);
        assert_eq!((name, outcome), ("tune", "discarded"));
        let reason = reason.unwrap();
        assert!(reason.contains("could not be compared"), "{reason}");
        assert!(reason.contains("after"), "{reason}");
        assert_eq!(held.kept, "full");
        // So did the input.
        let (_fixture, held) = tune_run("(pcb board)", &tuned_base(), "", &["A"], 30).await;
        let held = held.unwrap();
        let (_, outcome, reason) = tune_row(&held);
        assert_eq!(outcome, "discarded");
        assert!(reason.unwrap().contains("before"));
        assert_eq!(held.kept, "full");
    }

    /// The hold sequence at Floor fallback, in `folder` of the fixture.
    async fn held_with(
        fixture: &Fixture,
        folder: &str,
        files: CallFiles<'_>,
        tune: Option<TuneInputs<'_>>,
    ) -> Held {
        route_held(
            &fixture.binary(),
            &fixture.path().join(folder),
            &fixture.dsn(),
            &floor_hints(),
            files,
            WidthFallback::Floor,
            tune,
            soon(60),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn routing_passes_never_get_tune_and_the_tune_pass_takes_its_own_heights() {
        let fixture = Fixture::new("");
        let session_path = fixture.path().join("any.ses");
        std::fs::write(&session_path, base()).unwrap();
        // Every pass writes the same valid session, so the Tune pass is kept.
        fixture.set_control(&format!(
            "copy_session={}\nunrouted.full=3\nunrouted.fallback=1\nunrouted.polish=1\nunrouted.tune=1\n",
            session_path.display()
        ));
        let (tune_file, pairs) = (
            fixture.path().join("tune.txt"),
            fixture.path().join("pairs.txt"),
        );
        std::fs::write(&tune_file, "group lt1 tolerance=0.5\n  A\n").unwrap();
        std::fs::write(&pairs, "pair /X /Y gap=0.2\n").unwrap();
        let (classes, nets) = (class_list(&["c1"]), names(&["A"]));
        let args_of_pass = |folder: &str, name: &str| -> Vec<String> {
            std::fs::read_to_string(
                fixture
                    .path()
                    .join(folder)
                    .join(name)
                    .join(format!("{name}.ses.args")),
            )
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
        };

        // A call with groups and a pair: routing passes get the pair's heights right
        // after --pairs and never --tune; the Tune pass gets its own heights.
        let held = held_with(
            &fixture,
            "with_pair",
            CallFiles {
                amps: None,
                pairs: Some(&pairs),
                layer_heights: Some("9,9"),
            },
            Some(tune_inputs(&tune_file, &classes, &nets, Some("0,1.58"))),
        )
        .await;
        assert_eq!(held.kept, TUNE_PASS, "{:?}", held.passes);
        for name in ["full", "fallback", "polish"] {
            let args = args_of_pass("with_pair", name);
            let at = |prefix: &str| args.iter().position(|t| t.starts_with(prefix));
            assert_eq!(value(&args, "--layer-heights="), Some("9,9"), "{name}");
            assert_eq!(
                at("--layer-heights="),
                at("--pairs=").map(|p| p + 1),
                "{name}"
            );
            assert!(
                !args.iter().any(|t| t.starts_with("--tune")),
                "{name}: {args:?}"
            );
            assert!(
                !args
                    .iter()
                    .any(|t| t.starts_with("--router.autorouter.ignore_net_classes")),
                "{name}: {args:?}"
            );
        }
        let args = args_of_pass("with_pair", "tune");
        assert_eq!(value(&args, "--layer-heights="), Some("0,1.58"));
        assert!(!args.iter().any(|t| t.starts_with("--pairs")));
        assert_eq!(
            value(&args, "--initial-session="),
            Some(
                fixture
                    .path()
                    .join("with_pair/polish/polish.ses")
                    .to_str()
                    .unwrap()
            ),
            "the Tune pass starts from the last kept session"
        );

        // A call with groups but no pair: the routing passes carry no heights, the
        // Tune pass does, and the routing lines are exactly what they were before
        // Row LT (the same command lines pass_arguments gives without the new input).
        let held = held_with(
            &fixture,
            "no_pair",
            CallFiles::default(),
            Some(tune_inputs(&tune_file, &classes, &nets, Some("0,1.58"))),
        )
        .await;
        assert_eq!(held.kept, TUNE_PASS);
        for (name, neck, optimizer, initial) in [
            ("full", 0.0, true, None),
            ("fallback", 120.0, false, Some("no_pair/full/full.ses")),
            ("polish", 0.0, true, Some("no_pair/fallback/fallback.ses")),
        ] {
            let folder = fixture.path().join("no_pair").join(name);
            let expected = pass_arguments(
                &fixture.dsn(),
                &folder.join(format!("{name}.ses")),
                &folder.join(format!("{name}.json")),
                &floor_hints(),
                CallFiles::default(),
                &Pass {
                    name: match name {
                        "full" => "full",
                        "fallback" => "fallback",
                        _ => "polish",
                    },
                    neck_width_um: neck,
                    optimizer,
                    initial_session: initial.map(|path| fixture.path().join(path)),
                },
            );
            assert_eq!(args_of_pass("no_pair", name), expected, "{name}");
        }
        assert_eq!(
            value(&args_of_pass("no_pair", "tune"), "--layer-heights="),
            Some("0,1.58")
        );

        // A call with no groups has no Tune pass at all.
        let held = held_with(&fixture, "no_groups", CallFiles::default(), None).await;
        assert_eq!(held.kept, "polish");
        assert!(!fixture.path().join("no_groups/tune").exists());
        assert!(held.passes.iter().all(|row| row["name"] != "tune"));
    }

    /// A real Tune pass, sandboxed as every pass is: the candidate build lengthens
    /// the short net of a two-net group and touches no other net, and with a window
    /// no net misses it changes nothing. Run by hand, never `--live`:
    ///
    /// ```text
    /// cargo test -p konnect-core --lib a_real_tune_pass -- --ignored --nocapture
    /// ```
    ///
    /// `KONNECT_FASTROUTE` names the binary (default: the candidate's place under
    /// `HOME`). The board is the public fixture `specctra_pours_routing`.
    #[tokio::test]
    #[ignore = "needs the candidate fastroute build (KONNECT_FASTROUTE or its default place); runs it sandboxed"]
    async fn a_real_tune_pass_lengthens_only_its_nets() {
        use crate::route_search::{variant_text, Shape};
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let dir = tempfile::tempdir().unwrap();
        let environment = std::env::var("KONNECT_FASTROUTE").ok();
        let binary = match locate(
            None,
            environment.as_deref(),
            None,
            None,
            home.as_deref(),
            &dir.path().join("probe"),
        )
        .await
        {
            Located::Found(binary) => binary,
            other => panic!("no usable fastroute: {other:?}"),
        };
        let fixture_dsn =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/specctra_pours_routing.dsn");
        let dsn_text = variant_text(
            &std::fs::read_to_string(fixture_dsn).unwrap(),
            Shape::AsExported,
            1,
        )
        .unwrap();
        let dsn = dir.path().join("variant.dsn");
        std::fs::write(&dsn, &dsn_text).unwrap();
        let classes = crate::specctra::dsn_class_names(&dsn_text).unwrap();
        assert!(!classes.is_empty());
        let nets = names(&["S1", "S2"]);
        let hint = RoutingHints {
            copper_edge_clearance_um: Some(300),
            retry_width_um: None,
            retry_off_reason: Some("a test"),
        };
        let wire_mm = |ses: &Path, net: &str| -> f64 {
            copper_by_net(&std::fs::read_to_string(ses).unwrap()).unwrap()[net]
                .pieces
                .iter()
                .map(|(_, _, [x1, y1, x2, y2])| {
                    (((x2 - x1) as f64).powi(2) + ((y2 - y1) as f64).powi(2)).sqrt() / 1000.0
                })
                .sum()
        };
        let mut lengths = Vec::new();
        for (name, tolerance) in [("wide", "30"), ("tight", "0.5")] {
            let tune_file = dir.path().join(format!("{name}.txt"));
            std::fs::write(
                &tune_file,
                format!("group lt1 tolerance={tolerance}\n  S1\n  S2\n"),
            )
            .unwrap();
            let held = route_held(
                &binary,
                &dir.path().join(name),
                &dsn,
                &hint,
                CallFiles::default(),
                WidthFallback::None,
                Some(tune_inputs(&tune_file, &classes, &nets, Some("0,1.58"))),
                soon(300),
            )
            .await
            .unwrap();
            let rows: Vec<_> = held.passes.iter().map(|r| r["outcome"].clone()).collect();
            let tune_row = held.passes.last().unwrap();
            assert_eq!(tune_row["name"], "tune", "{rows:?}");
            assert_eq!(tune_row["outcome"], "kept", "{tune_row}");
            assert_eq!(held.kept, TUNE_PASS);
            let full = dir.path().join(name).join("full/full.ses");
            assert!(changed_outside(
                &std::fs::read_to_string(&full).unwrap(),
                &std::fs::read_to_string(&held.ses).unwrap(),
                &nets
            )
            .unwrap()
            .is_empty());
            let (before, after) = (wire_mm(&full, "S2"), wire_mm(&held.ses, "S2"));
            eprintln!(
                "{name}: S2 {before:.2} -> {after:.2} mm, S1 {:.2} -> {:.2}",
                wire_mm(&full, "S1"),
                wire_mm(&held.ses, "S1")
            );
            lengths.push((
                before,
                after,
                wire_mm(&full, "S1"),
                wire_mm(&held.ses, "S1"),
            ));
        }
        // A window no net misses changes the geometry of nothing; a tight one
        // lengthens the short net and leaves the longest alone.
        let (before, after, ..) = lengths[0];
        assert!((before - after).abs() < 1e-3, "{lengths:?}");
        let (before, after, s1_before, s1_after) = lengths[1];
        assert!(after > before + 5.0, "{lengths:?}");
        assert!((s1_before - s1_after).abs() < 1e-3, "{lengths:?}");
    }
}
