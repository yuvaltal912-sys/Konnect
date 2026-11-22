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

/// No via padstack has a known disc: [`changed_outside`] then ignores nothing.
static NO_VIA_RADII: BTreeMap<String, i64> = BTreeMap::new();

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
        planes: &[],
        via_radii: &NO_VIA_RADII,
    }
}

/// The next round's tune file when no group can be written (these tests have no
/// plan): a pass that touches itself is then discarded.
fn no_next(
    _folder: &Path,
    _dropped: &BTreeSet<String>,
) -> Result<Option<(PathBuf, BTreeSet<String>)>> {
    Ok(None)
}

/// A request without a next round.
fn tune_request(inputs: TuneInputs<'_>) -> TuneRequest<'_> {
    TuneRequest {
        inputs,
        next: &no_next,
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
        TuneVariant::Free,
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
            "--router.autorouter.enabled=false",
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
        names(&[
            "--router.autorouter.enabled",
            "--router.autorouter.ignore_net_classes",
            "--tune"
        ]),
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
    changed_outside(before, after, &names(nets), &NO_VIA_RADII).unwrap()
}

/// [`changed`] with the via discs of a DSN.
fn changed_with(
    before: &str,
    after: &str,
    nets: &[&str],
    via_radii: &BTreeMap<String, i64>,
) -> Vec<String> {
    changed_outside(before, after, &names(nets), via_radii).unwrap()
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
    // (A `qarc` is skipped, not an error: see `a_qarc_is_skipped_...`.)
    let not_a_path = session(&[("A", "(wire (polygon F.Cu 2500 0 0 10 0 0 10))".to_string())]);
    for (what, text) in [
        ("not a session", "(pcb board)".to_string()),
        ("not balanced", "(session b (routes".to_string()),
        ("a wire that is not a path", not_a_path),
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
        let error = changed_outside(&text, &base(), &names(&[]), &NO_VIA_RADII).unwrap_err();
        assert!(format!("{error:#}").contains("before"), "{what}: {error:#}");
        let error = changed_outside(&base(), &text, &names(&[]), &NO_VIA_RADII).unwrap_err();
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

/// A raw wire in session units (not micrometres), so a coordinate need not be a whole
/// micrometre.
fn raw_wire(points: &[(i64, i64)]) -> String {
    let coordinates: Vec<String> = points
        .iter()
        .flat_map(|(x, y)| [x.to_string(), y.to_string()])
        .collect();
    format!("(wire (path F.Cu 2500 {}))", coordinates.join(" "))
}

#[test]
fn a_diagonal_split_at_different_vertices_is_the_same_copper() {
    // Slope 1:3, odd session units: rounded to 1 um first, the pieces would no longer
    // lie on one line and would not merge.
    let whole = session(&[("A", raw_wire(&[(1, 2), (1003, 336)]))]);
    let split = session(&[("A", raw_wire(&[(1, 2), (16, 7), (1003, 336)]))]);
    let split_twice = session(&[(
        "A",
        raw_wire(&[(1, 2), (16, 7)]) + &raw_wire(&[(1003, 336), (301, 102), (16, 7)]),
    )]);
    assert!(changed(&whole, &split, &[]).is_empty());
    assert!(changed(&split, &split_twice, &[]).is_empty());
    assert_eq!(
        copper_by_net(&whole).unwrap(),
        copper_by_net(&split).unwrap()
    );
    // Only the merged piece's ends are rounded: (0.1, 0.2) and (100.3, 33.6) um.
    assert_eq!(
        copper_by_net(&split).unwrap()["A"].pieces,
        vec![("F.Cu".to_string(), 250, [0, 0, 100, 34])]
    );
    // Negative control: the same diagonal shifted by 2 um is other copper.
    let shifted = session(&[(
        "A",
        raw_wire(&[(21, 2), (36, 7)]) + &raw_wire(&[(36, 7), (1023, 336)]),
    )]);
    assert_eq!(changed(&whole, &shifted, &[]), vec!["A".to_string()]);
}

/// The pieces of net `A` for wires given as lists of points (micrometres), all on one
/// layer and width.
fn pieces_of(wires: &[Vec<(i64, i64)>]) -> Vec<(String, i64, [i64; 4])> {
    let text: String = wires
        .iter()
        .map(|points| wire("F.Cu", 250, points))
        .collect();
    copper_by_net(&session(&[("A", text)])).unwrap()["A"]
        .pieces
        .clone()
}

/// Both ends of a piece, ordered, so a test does not depend on which end is first.
fn ends_of(piece: &(String, i64, [i64; 4])) -> [(i64, i64); 2] {
    let mut ends = [(piece.2[0], piece.2[1]), (piece.2[2], piece.2[3])];
    ends.sort();
    ends
}

/// Collinear pieces merge exactly when they touch or overlap on one line (L6): on every
/// direction, slope and orientation, and a parallel piece or a gap never merges.
#[test]
fn collinear_pieces_merge_on_every_direction_and_only_when_they_touch_or_overlap() {
    // A step of the line in micrometres; the gcd of its parts is above 1 for several,
    // so the pieces' own direction vectors differ in size while the line is one.
    for step in [
        (30, 0),
        (0, 40),
        (20, 20),
        (20, -20),
        (30, 60),
        (60, -30),
        (-40, 10),
        (-15, -45),
        (10, 30),
    ] {
        let start = (700, 1100);
        let at = |k: i64| (start.0 + k * step.0, start.1 + k * step.1);
        // The same step turned a quarter: a parallel line, with the same positions along
        // the line.
        let aside = |k: i64| {
            let (x, y) = at(k);
            (x - step.1, y + step.0)
        };
        let whole = pieces_of(&[vec![at(0), at(6)]]);
        assert_eq!(whole.len(), 1, "{step:?}");
        assert_eq!(ends_of(&whole[0]), {
            let mut ends = [at(0), at(6)];
            ends.sort();
            ends
        });
        for (what, wires) in [
            // Touching, with unequal lengths and either orientation, either order.
            ("touching", vec![vec![at(0), at(2)], vec![at(2), at(6)]]),
            (
                "touching, reversed",
                vec![vec![at(6), at(2)], vec![at(2), at(0)]],
            ),
            (
                "touching, mixed",
                vec![vec![at(0), at(2)], vec![at(6), at(2)]],
            ),
            (
                "touching, mixed the other way",
                vec![vec![at(2), at(0)], vec![at(2), at(6)]],
            ),
            (
                "touching, listed last first",
                vec![vec![at(2), at(6)], vec![at(0), at(2)]],
            ),
            // Overlapping.
            ("overlapping", vec![vec![at(0), at(4)], vec![at(3), at(6)]]),
            (
                "overlapping, mixed",
                vec![vec![at(4), at(0)], vec![at(3), at(6)]],
            ),
            // One inside another.
            ("contained", vec![vec![at(0), at(6)], vec![at(2), at(3)]]),
            (
                "contained, listed after",
                vec![vec![at(2), at(3)], vec![at(6), at(0)]],
            ),
            // One polyline that doubles back.
            ("doubling back", vec![vec![at(0), at(6), at(2)]]),
            // Three pieces, the middle one listed last.
            (
                "three",
                vec![vec![at(0), at(1)], vec![at(4), at(6)], vec![at(1), at(4)]],
            ),
        ] {
            assert_eq!(pieces_of(&wires), whole, "{step:?}: {what}");
        }
        // A gap leaves two pieces, one on each side of it.
        let gapped = pieces_of(&[vec![at(0), at(2)], vec![at(4), at(6)]]);
        assert_eq!(gapped.len(), 2, "{step:?}: a gap");
        let mut found: Vec<_> = gapped.iter().map(ends_of).collect();
        found.sort();
        let ordered = |a: (i64, i64), b: (i64, i64)| {
            let mut ends = [a, b];
            ends.sort();
            ends
        };
        let mut want = vec![ordered(at(0), at(2)), ordered(at(4), at(6))];
        want.sort();
        assert_eq!(found, want, "{step:?}: a gap");
        // A piece beside the line never merges, touching in position along it or not.
        for (what, wires) in [
            (
                "beside, touching",
                vec![vec![at(0), at(3)], vec![aside(3), aside(6)]],
            ),
            (
                "beside, overlapping",
                vec![vec![at(0), at(6)], vec![aside(0), aside(6)]],
            ),
            (
                "beside, inside",
                vec![vec![at(0), at(6)], vec![aside(2), aside(3)]],
            ),
        ] {
            let pieces = pieces_of(&wires);
            assert_eq!(pieces.len(), 2, "{step:?}: {what}");
        }
    }
}

#[test]
fn a_piece_of_no_length_is_no_copper() {
    let degenerate = |points: &[(i64, i64)]| pieces_of(&[points.to_vec()]);
    // A repeated point adds nothing to the piece.
    assert_eq!(
        degenerate(&[(100, 100), (100, 100), (500, 100)]),
        degenerate(&[(100, 100), (500, 100)])
    );
    assert_eq!(degenerate(&[(100, 100), (500, 100), (500, 100)]).len(), 1);
    // A path that never leaves its point is no piece at all.
    assert!(degenerate(&[(100, 100), (100, 100)]).is_empty());
    let alone = session(&[("A", wire("F.Cu", 250, &[(100, 100), (100, 100)]))]);
    assert!(changed(&alone, &session(&[]), &[]).is_empty());
}

// ---- Row LT, L20 and L24: the router switch and the pour-safe switch ---------

/// The Tune line of a candidate, with the given classes.
fn variant_args(variant: TuneVariant, classes: &[String]) -> Vec<String> {
    let nets = names(&["A"]);
    tune_pass_arguments(
        Path::new("/d/in.dsn"),
        Path::new("/d/out.ses"),
        Path::new("/d/out.json"),
        &hints(Some(500), Some(120)),
        CallFiles {
            amps: Some(Path::new("/d/amps.txt")),
            pairs: None,
            layer_heights: Some("0,1.58"),
        },
        variant,
        &tune_inputs(Path::new("/d/tune.txt"), classes, &nets, Some("0,1.58")),
        Path::new("/d/held.ses"),
    )
}

#[test]
fn the_tune_line_turns_the_autorouter_off_once_right_after_the_optimizer_switch() {
    assert_eq!(TUNE_ROUTER_OFF, "--router.autorouter.enabled=false");
    for variant in [TuneVariant::Free, TuneVariant::PourSafe] {
        for classes in [class_list(&[]), class_list(&["c1", "c2"])] {
            let args = variant_args(variant, &classes);
            let at = |token: &str| args.iter().position(|arg| arg == token);
            assert_eq!(
                args.iter().filter(|arg| *arg == TUNE_ROUTER_OFF).count(),
                1,
                "{variant:?}: {args:?}"
            );
            assert_eq!(
                at(TUNE_ROUTER_OFF),
                at("--router.optimizer.enabled=false").map(|optimizer| optimizer + 1),
                "{variant:?}: right after the optimizer switch"
            );
            // The class list stays as a second lock, and no other setting reads the
            // autorouter's switch.
            assert_eq!(
                value(&args, "--router.autorouter.ignore_net_classes=").is_some(),
                !classes.is_empty()
            );
            assert_eq!(
                args.iter()
                    .filter(|arg| arg.starts_with("--router.autorouter.enabled"))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn no_routing_pass_line_ever_holds_the_tune_switches() {
    for pass in [
        pass("full", 0.0, true, None),
        pass("fallback", 120.0, false, Some("/d/full.ses")),
        pass("polish", 0.0, true, Some("/d/fallback.ses")),
    ] {
        let args = args_with(
            &pass,
            &hints(Some(250), Some(120)),
            CallFiles {
                amps: Some(Path::new("/d/amps.txt")),
                pairs: Some(Path::new("/d/pairs.txt")),
                layer_heights: Some("0,1.58"),
            },
        );
        for token in &args {
            assert!(
                !token.starts_with("--router.autorouter.enabled")
                    && !token.starts_with("--router.plane_as_obstacle"),
                "{}: {token}",
                pass.name
            );
        }
        assert!(!args
            .iter()
            .any(|t| t == TUNE_ROUTER_OFF || t == TUNE_POUR_SAFE));
    }
}

#[test]
fn only_the_pour_safe_line_holds_the_plane_switch_once() {
    assert_eq!(TUNE_POUR_SAFE, "--router.plane_as_obstacle=true");
    let classes = class_list(&["c1"]);
    let free = variant_args(TuneVariant::Free, &classes);
    let safe = variant_args(TuneVariant::PourSafe, &classes);
    assert!(
        !free
            .iter()
            .any(|arg| arg.starts_with("--router.plane_as_obstacle")),
        "{free:?}"
    );
    assert_eq!(safe.iter().filter(|arg| *arg == TUNE_POUR_SAFE).count(), 1);
    // The pour-safe line is the free line plus that one token, after the router switch.
    let mut expected = free.clone();
    let at = expected
        .iter()
        .position(|arg| arg == TUNE_ROUTER_OFF)
        .unwrap();
    expected.insert(at + 1, TUNE_POUR_SAFE.to_string());
    assert_eq!(safe, expected);
}

#[test]
fn a_tune_candidates_folder_names_its_variant_and_round() {
    assert_eq!(TuneVariant::Free.folder_name(1), "tune");
    assert_eq!(TuneVariant::Free.folder_name(2), "tune-r2");
    assert_eq!(TuneVariant::Free.folder_name(3), "tune-r3");
    assert_eq!(TuneVariant::PourSafe.folder_name(1), "tune-pour-safe");
    assert_eq!(TuneVariant::PourSafe.folder_name(2), "tune-pour-safe-r2");
    // Round 1 is the unsuffixed folder of today's Tune pass.
    assert_eq!(TuneVariant::Free.folder_name(1), TUNE_PASS);
    assert_eq!(
        serde_json::to_value(TuneVariant::PourSafe).unwrap(),
        json!("pour_safe")
    );
    assert_eq!(
        serde_json::to_value(TuneVariant::Free).unwrap(),
        json!("free")
    );
}

#[test]
fn every_tune_pass_folder_a_run_can_hold_is_named_once() {
    // L26 reduces and releases exactly these.
    assert_eq!(
        tune_folder_names(),
        [
            "tune",
            "tune-r2",
            "tune-r3",
            "tune-pour-safe",
            "tune-pour-safe-r2",
            "tune-pour-safe-r3"
        ]
    );
    assert_eq!(TUNE_ROUNDS_MAX, 3, "the list follows the rounds");
}

// ---- the Tune pass's guard and a via stub (M4, decision 7) ------------------------

/// M4's board B, run 6: net `C_N` of `full/full.ses` and of `tune-r2/tune.ses` (the
/// session round 2 wrote, with `C_N` outside its tune file), and the via padstack of that
/// run's DSN.
const M4_B6_HELD: FixtureText = include_str!("../tests/fixtures/length_tuning/m4_b6_c_n_held.ses");
const M4_B6_ROUND_2: FixtureText =
    include_str!("../tests/fixtures/length_tuning/m4_b6_c_n_tune_r2.ses");
const M4_B6_DSN: FixtureText = include_str!("../tests/fixtures/length_tuning/m4_b6_vias.dsn");

#[test]
fn the_via_discs_of_a_dsn_are_the_smallest_copper_radius_of_each_padstack() {
    let radii = crate::specctra::via_disc_radii(M4_B6_DSN);
    // 450 um circles on six layers: a radius of 225 um.
    assert_eq!(
        radii,
        BTreeMap::from([("konnect_via_0001".to_string(), 225)])
    );
    let dsn = |library: &str, unit: &str| {
        format!("(pcb b (resolution um 10) (unit {unit}) (library {library}))")
    };
    let stack = |name: &str, shapes: &str| format!("(padstack {name} {shapes} (attach off))");
    let circles = |first: &str, second: &str| {
        format!("(shape (circle F.Cu {first})) (shape (circle B.Cu {second}))")
    };
    // The smallest circle, rounded down.
    let found = crate::specctra::via_disc_radii(&dsn(&stack("a", &circles("450", "401")), "um"));
    assert_eq!(found["a"], 200);
    // Millimetres are read as such.
    let found = crate::specctra::via_disc_radii(&dsn(&stack("a", &circles("0.6", "0.6")), "mm"));
    assert_eq!(found["a"], 300);
    // No entry: a shape that is not a circle, a diameter that is not positive, another
    // unit, an unreadable DSN. The guard is then strict.
    let mixed = stack(
        "m",
        "(shape (circle F.Cu 450)) (shape (rect F.Cu 0 0 10 10))",
    );
    assert!(crate::specctra::via_disc_radii(&dsn(&mixed, "um")).is_empty());
    assert!(
        crate::specctra::via_disc_radii(&dsn(&stack("z", &circles("0", "450")), "um")).is_empty()
    );
    assert!(
        crate::specctra::via_disc_radii(&dsn(&stack("a", &circles("450", "450")), "mil"))
            .is_empty()
    );
    assert!(crate::specctra::via_disc_radii("(pcb b").is_empty());
    assert!(crate::specctra::via_disc_radii("(session b)").is_empty());
}

#[test]
fn m4s_board_b_run_6_compares_equal_with_the_via_stub_the_load_dropped() {
    // Round 2 of the free candidate wrote `C_N` (outside its tune file) with its 20 um
    // In1.Cu stub at a via missing and its 0.212 mm bridge three times more: the
    // stub is the only piece that differs.
    let radii = crate::specctra::via_disc_radii(M4_B6_DSN);
    let outside = ["/CM5/DPHY0_P.D0_P", "/CM5/DPHY0_P.D0_N"];
    assert!(changed_with(M4_B6_HELD, M4_B6_ROUND_2, &outside, &radii).is_empty());
    // Control: without the via discs the same pair of sessions names the net, as before.
    assert_eq!(
        changed(M4_B6_HELD, M4_B6_ROUND_2, &outside),
        ["/CM5/DPHY0_P.C_N"]
    );
    // Only a missing piece is ignored: the other way round the stub is a new piece.
    assert_eq!(
        changed_with(M4_B6_ROUND_2, M4_B6_HELD, &outside, &radii),
        ["/CM5/DPHY0_P.C_N"]
    );
    // The net inside the tune file is never compared.
    assert!(changed_with(M4_B6_HELD, M4_B6_ROUND_2, &["/CM5/DPHY0_P.C_N"], &radii).is_empty());
}

/// Net `A`: a trace, a via `v` at (1000, 1000) um, and on In1.Cu a stub from the via to
/// (1000, 1000 + `reach`).
fn net_with_stub(reach: i64, via_net: &str) -> String {
    let stub = wire("In1.Cu", 200, &[(1000, 1000), (1000, 1000 + reach)]);
    let trace = wire("F.Cu", 200, &[(0, 1000), (1000, 1000)]);
    let via_item = via("v", 1000, 1000);
    if via_net == "A" {
        session(&[
            ("A", trace + &via_item + &stub),
            ("B", via("w", 5000, 5000)),
        ])
    } else {
        session(&[("A", trace + &stub), ("B", via_item)])
    }
}

#[test]
fn a_missing_piece_is_a_via_stub_only_inside_the_disc_of_a_via_of_its_own_net() {
    let radii = BTreeMap::from([("v".to_string(), 225), ("w".to_string(), 225)]);
    let after = session(&[
        (
            "A",
            wire("F.Cu", 200, &[(0, 1000), (1000, 1000)]) + &via("v", 1000, 1000),
        ),
        ("B", via("w", 5000, 5000)),
    ]);
    // Both ends within the disc, one on its edge (225 um): ignored.
    assert!(changed_with(&net_with_stub(225, "A"), &after, &[], &radii).is_empty());
    assert!(changed_with(&net_with_stub(20, "A"), &after, &[], &radii).is_empty());
    // One micrometre past the disc: named.
    assert_eq!(
        changed_with(&net_with_stub(226, "A"), &after, &[], &radii),
        ["A"]
    );
    // Inside a via of another net: named. (The via sits on net B in both sessions.)
    let after_b = session(&[
        ("A", wire("F.Cu", 200, &[(0, 1000), (1000, 1000)])),
        ("B", via("v", 1000, 1000)),
    ]);
    assert_eq!(
        changed_with(&net_with_stub(20, "B"), &after_b, &[], &radii),
        ["A"]
    );
    // A via whose padstack has no known disc: named.
    assert_eq!(
        changed_with(&net_with_stub(20, "A"), &after, &[], &BTreeMap::new()),
        ["A"]
    );
    // A smaller disc than the stub reaches: named.
    let small = BTreeMap::from([("v".to_string(), 10)]);
    assert_eq!(
        changed_with(&net_with_stub(20, "A"), &after, &[], &small),
        ["A"]
    );
    // A stub that starts outside the disc and ends inside it: named (both ends count).
    let outside_start = session(&[
        (
            "A",
            wire("F.Cu", 200, &[(0, 1000), (1000, 1000)])
                + &via("v", 1000, 1000)
                + &wire("In1.Cu", 200, &[(1000, 1300), (1000, 1100)]),
        ),
        ("B", via("w", 5000, 5000)),
    ]);
    assert_eq!(changed_with(&outside_start, &after, &[], &radii), ["A"]);
}

#[test]
fn a_trace_between_two_vias_of_one_net_is_not_a_stub() {
    // Net A has vias `v` and `v2` 700 um apart, and an In1.Cu trace between their
    // centres: each end is inside the disc of one via, but no one via holds both.
    let radii = BTreeMap::from([("v".to_string(), 225), ("v2".to_string(), 225)]);
    let vias = via("v", 1000, 1000) + &via("v2", 1700, 1000);
    let with_trace = session(&[(
        "A",
        vias.clone() + &wire("In1.Cu", 200, &[(1000, 1000), (1700, 1000)]),
    )]);
    let without = session(&[("A", vias)]);
    assert_eq!(changed_with(&with_trace, &without, &[], &radii), ["A"]);
    // Control: a stub of the same net inside one of the two discs is still ignored.
    let with_stub = session(&[(
        "A",
        via("v", 1000, 1000)
            + &via("v2", 1700, 1000)
            + &wire("In1.Cu", 200, &[(1700, 1000), (1700, 1100)]),
    )]);
    assert!(changed_with(&with_stub, &without, &[], &radii).is_empty());
}

#[test]
fn a_via_stub_does_not_excuse_a_changed_via_or_a_new_piece() {
    let radii = BTreeMap::from([("v".to_string(), 225)]);
    let held = net_with_stub(20, "A");
    // The stub is missing and a via moved: named.
    let moved = session(&[
        (
            "A",
            wire("F.Cu", 200, &[(0, 1000), (1000, 1000)]) + &via("v", 1001, 1000),
        ),
        ("B", via("w", 5000, 5000)),
    ]);
    assert_eq!(changed_with(&held, &moved, &[], &radii), ["A"]);
    // The stub is missing and another piece appeared: named.
    let added = session(&[
        (
            "A",
            wire("F.Cu", 200, &[(0, 1000), (1000, 1000)])
                + &via("v", 1000, 1000)
                + &wire("F.Cu", 200, &[(0, 3000), (500, 3000)]),
        ),
        ("B", via("w", 5000, 5000)),
    ]);
    assert_eq!(changed_with(&held, &added, &[], &radii), ["A"]);
    // Control: the same sessions with nothing else different are equal.
    assert!(changed_with(&held, &held, &[], &radii).is_empty());
}

// ---- Row LT, L21: self contacts ----------------------------------------------

type FixtureText = &'static str;

const A_DQ10_TUNED: FixtureText = include_str!("../tests/fixtures/length_tuning/a_dq10_tuned.ses");
const A_DQ8_CROSSING: FixtureText =
    include_str!("../tests/fixtures/length_tuning/a_dq8_crossing.ses");
const A_CLEAN: FixtureText = include_str!("../tests/fixtures/length_tuning/a_clean_dq2_dq16.ses");
const A_DQ0_HELD: FixtureText = include_str!("../tests/fixtures/length_tuning/a_dq0_held.ses");
const A_DQ0_TUNED: FixtureText = include_str!("../tests/fixtures/length_tuning/a_dq0_tuned.ses");
const B_C_N_HELD: FixtureText = include_str!("../tests/fixtures/length_tuning/b_c_n_held.ses");
const B_C_N_TUNED: FixtureText = include_str!("../tests/fixtures/length_tuning/b_c_n_tuned.ses");
const B_D1_P_HELD: FixtureText = include_str!("../tests/fixtures/length_tuning/b_d1_p_held.ses");
const B_D1_P_TUNED: FixtureText = include_str!("../tests/fixtures/length_tuning/b_d1_p_tuned.ses");
const B_FB_TUNED: FixtureText = include_str!("../tests/fixtures/length_tuning/b_fb_tuned.ses");
const B_C_N: &str = "/CM5/DPHY0_P.C_N";

/// A piece as a [`SelfContact`] names it: width and ends in µm.
fn piece(width_um: i64, ends: [i64; 4]) -> (i64, [i64; 4]) {
    (width_um, ends)
}

/// The self contacts of net `A` of a session made of `items`.
fn contacts_of(items: String) -> Vec<SelfContact> {
    self_contacts(&session(&[("A", items)]), &names(&["A"]))
        .unwrap()
        .remove("A")
        .unwrap_or_default()
}

/// The kinds of those contacts.
fn kinds_of(items: String) -> Vec<ContactKind> {
    contacts_of(items).iter().map(|c| c.kind).collect()
}

fn kind_count(contacts: &[SelfContact], kind: ContactKind) -> usize {
    contacts.iter().filter(|c| c.kind == kind).count()
}

#[test]
fn two_distinct_collinear_pieces_overlap_and_name_both_pieces_rounded_and_ordered() {
    // /DQ10 of A-tuned: its wire holds In1.Cu pieces at y = -100.4201 mm (session
    // units of 0.1 µm): segment 71, (1626829 to 1646829), and segment 77, (1620829 to
    // 1649012), a partial collinear overlap. Rounded to 1 µm.
    let found = self_contacts(A_DQ10_TUNED, &names(&["/DQ10"])).unwrap();
    let contacts = &found["/DQ10"];
    assert!(
        contacts.contains(&SelfContact {
            layer: "In1.Cu".to_string(),
            kind: ContactKind::Overlap,
            pieces: [
                piece(200, [162083, -100420, 164901, -100420]),
                piece(200, [162683, -100420, 164683, -100420]),
            ],
        }),
        "segments 71 and 77"
    );
    // Segments 51 and 143 of the same wire are an exact repeat: one piece, so no
    // contact holds the same piece twice.
    assert!(contacts.iter().all(|c| c.pieces[0] != c.pieces[1]));
    // Counted pair by pair after the exact repeats are merged, as an independent
    // script does: 148 crossings, 125 overlaps, 286 touches.
    assert_eq!(kind_count(contacts, ContactKind::Crossing), 148);
    assert_eq!(kind_count(contacts, ContactKind::Overlap), 125);
    assert_eq!(kind_count(contacts, ContactKind::Touch), 286);
    // Two pieces on the same stretch, given in either order and direction, are the
    // same contact: the pair is ordered and so are the ends.
    let reversed = contacts_of(
        wire("F.Cu", 200, &[(1000, 0), (0, 0)]) + &wire("F.Cu", 200, &[(500, 0), (1500, 0)]),
    );
    assert_eq!(
        reversed,
        vec![SelfContact {
            layer: "F.Cu".to_string(),
            kind: ContactKind::Overlap,
            pieces: [piece(200, [0, 0, 1000, 0]), piece(200, [500, 0, 1500, 0])],
        }]
    );
}

#[test]
fn exact_repeats_are_one_piece_and_never_a_contact() {
    // /DQ10 segments 51 and 143: the same In1.Cu piece twice. The session holds it
    // twice (a piece alone that way has no contact at all):
    let twice = "1602829 -1004201\n            1632829 -1004201";
    assert!(
        A_DQ10_TUNED.matches(twice).count() >= 2,
        "the repeat is in the fixture"
    );
    let one = session(&[("A", wire("F.Cu", 200, &[(0, 0), (1000, 0)]))]);
    let many =
        |copies: usize| session(&[("A", wire("F.Cu", 200, &[(0, 0), (1000, 0)]).repeat(copies))]);
    for copies in [2, 3, 4] {
        assert!(self_contacts(&many(copies), &names(&["A"]))
            .unwrap()
            .is_empty());
    }
    assert!(self_contacts(&one, &names(&["A"])).unwrap().is_empty());
    // Either direction is the same piece.
    assert!(kinds_of(
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(1000, 0), (0, 0)])
    )
    .is_empty());
    // M1's C_N piece in four copies, and /PCIe-M2/FB's copied bridge, in B's step 0b
    // Tune session: one piece each, no contact (C_N keeps its two held touches).
    assert!(
        B_C_N_TUNED
            .matches("1247100 -453274\n            1247100 -451150")
            .count()
            >= 3
    );
    let found = self_contacts(B_C_N_TUNED, &names(&[B_C_N])).unwrap();
    assert!(found[B_C_N].iter().all(|c| c.kind == ContactKind::Touch));
    assert_eq!(found[B_C_N].len(), 2);
    assert!(B_FB_TUNED.matches("1273150 -714600").count() >= 2);
    assert!(self_contacts(B_FB_TUNED, &names(&["/PCIe-M2/FB"]))
        .unwrap()
        .is_empty());
    // The key is layer, width and ends: the same ends with another width or on
    // another layer are not a repeat (the first overlaps; the other is none).
    assert_eq!(
        kinds_of(
            wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 150, &[(0, 0), (1000, 0)])
        ),
        [ContactKind::Overlap]
    );
    assert!(kinds_of(
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("B.Cu", 200, &[(0, 0), (1000, 0)])
    )
    .is_empty());
    // A piece nudged by 1 µm is not the same piece: the pair is an overlap.
    assert_eq!(
        kinds_of(
            wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(1, 0), (1000, 0)])
        ),
        [ContactKind::Overlap]
    );
}

#[test]
fn centre_lines_that_cross_are_a_crossing_on_one_layer_only_and_for_one_net() {
    // A crossing pair of A-tuned's /DQ8, cut from its tune session.
    let found = self_contacts(A_DQ8_CROSSING, &names(&["/DQ8"])).unwrap();
    assert_eq!(found["/DQ8"].len(), 1);
    assert_eq!(found["/DQ8"][0].kind, ContactKind::Crossing);
    assert_eq!(found["/DQ8"][0].layer, "F.Cu");
    assert_eq!(
        found["/DQ8"][0].pieces,
        [
            piece(200, [147185, -108478, 147609, -108054]),
            piece(200, [147420, -108443, 147420, -107443]),
        ]
    );
    // The same cross on two layers is no contact; on one layer it is.
    let cross_on = |second: &str| {
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire(second, 200, &[(500, -500), (500, 500)])
    };
    assert_eq!(kinds_of(cross_on("F.Cu")), [ContactKind::Crossing]);
    assert!(kinds_of(cross_on("B.Cu")).is_empty());
    // A crossing of two nets is no self contact.
    let two = session(&[
        ("A", wire("F.Cu", 200, &[(0, 0), (1000, 0)])),
        ("B", wire("F.Cu", 200, &[(500, -500), (500, 500)])),
    ]);
    assert!(self_contacts(&two, &names(&["A", "B"])).unwrap().is_empty());
    // A net that is not asked for is not read.
    let both = session(&[
        ("A", wire("F.Cu", 200, &[(0, 0), (1000, 0), (1000, 800)])),
        (
            "B",
            wire("F.Cu", 200, &[(0, 0), (1000, 0)])
                + &wire("F.Cu", 200, &[(500, -500), (500, 500)]),
        ),
    ]);
    assert!(self_contacts(&both, &names(&["A"])).unwrap().is_empty());
    assert_eq!(self_contacts(&both, &names(&["A", "B"])).unwrap().len(), 1);
    assert!(self_contacts(&both, &names(&[])).unwrap().is_empty());
    assert!(self_contacts(&both, &names(&["C"])).unwrap().is_empty());
}

#[test]
fn nets_without_a_contact_are_left_out() {
    // /DQ2 and /DQ16 of A-tuned: two of the 12 clean nets.
    assert!(self_contacts(A_CLEAN, &names(&["/DQ2", "/DQ16"]))
        .unwrap()
        .is_empty());
    // Control: the scanner is not blind to a net of the same session shape.
    assert!(self_contacts(A_DQ10_TUNED, &names(&["/DQ10"]))
        .unwrap()
        .contains_key("/DQ10"));
}

#[test]
fn joins_corners_and_end_to_end_pieces_are_no_contact() {
    // A T-junction: one piece's end on the other.
    assert!(kinds_of(
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(500, 0), (500, 600)])
    )
    .is_empty());
    // The junction from either side.
    assert!(kinds_of(
        wire("F.Cu", 200, &[(500, 0), (500, 600)]) + &wire("F.Cu", 200, &[(0, 0), (1000, 0)])
    )
    .is_empty());
    // A plain corner, and a 45 degree corner (the two pieces' copper overlaps near
    // the corner, but they share an end).
    assert!(kinds_of(wire("F.Cu", 200, &[(0, 0), (1000, 0), (1000, 1000)])).is_empty());
    assert!(kinds_of(wire("F.Cu", 200, &[(0, 0), (1000, 0), (1500, 500)])).is_empty());
    // Pieces that meet end to end on one line.
    assert!(kinds_of(wire("F.Cu", 200, &[(0, 0), (1000, 0), (2000, 0)])).is_empty());
    assert!(kinds_of(
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(1000, 0), (2000, 0)])
    )
    .is_empty());
    // A piece of no length is no copper.
    assert!(kinds_of(wire("F.Cu", 200, &[(0, 0), (0, 0), (1000, 0)])).is_empty());
    // Control: a corner whose far piece folds back over the first one does overlap,
    // although they share an end.
    assert_eq!(
        kinds_of(wire("F.Cu", 200, &[(0, 0), (1000, 0), (500, 0)])),
        [ContactKind::Overlap]
    );
}

#[test]
fn a_shared_stretch_is_an_overlap_only_when_longer_than_a_micrometre() {
    let on_line = |second_start: i64| {
        wire("F.Cu", 200, &[(0, 0), (1000, 0)])
            + &wire("F.Cu", 200, &[(second_start, 0), (second_start + 1000, 0)])
    };
    // 500 µm, 2 µm shared: overlaps.
    assert_eq!(kinds_of(on_line(500)), [ContactKind::Overlap]);
    assert_eq!(kinds_of(on_line(998)), [ContactKind::Overlap]);
    // 1 µm shared is not longer than the margin; the pieces are 0 apart, a touch.
    assert_eq!(kinds_of(on_line(999)), [ContactKind::Touch]);
    // No stretch shared and a gap smaller than the widths allow: a touch; wider: none.
    assert_eq!(kinds_of(on_line(1100)), [ContactKind::Touch]);
    assert!(kinds_of(on_line(1300)).is_empty());
    // Pieces of different widths on one line overlap too.
    assert_eq!(
        kinds_of(
            wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 150, &[(200, 0), (800, 0)])
        ),
        [ContactKind::Overlap]
    );
    // The same on a slanted line (3:4, so the stretch is exact): the pieces are in
    // session units of 0.1 µm. Sharing 1.5 µm is an overlap, sharing 1.0 µm is not.
    let slanted = |back: i64| {
        let raw = |x1: i64, y1: i64, x2: i64, y2: i64| {
            format!("(wire (path F.Cu 2000 {x1} {y1} {x2} {y2}))")
        };
        session_with(
            "um 10",
            &[(
                "A",
                raw(0, 0, 30000, 40000) + &raw(30000 - 3 * back, 40000 - 4 * back, 30300, 40400),
            )],
        )
    };
    let slanted_kinds = |back: i64| -> Vec<ContactKind> {
        self_contacts(&slanted(back), &names(&["A"]))
            .unwrap()
            .remove("A")
            .unwrap_or_default()
            .iter()
            .map(|contact| contact.kind)
            .collect()
    };
    assert_eq!(slanted_kinds(3), [ContactKind::Overlap]);
    assert_eq!(slanted_kinds(2), [ContactKind::Touch]);
    assert_eq!(slanted_kinds(300), [ContactKind::Overlap]);
    // Overlap is not read off a different line.
    assert!(kinds_of(
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(0, 600), (1000, 600)])
    )
    .is_empty());
}

#[test]
fn a_touch_is_closer_than_half_the_widths_minus_a_micrometre() {
    let legs = |width_a: i64, width_b: i64, gap: i64| {
        wire("F.Cu", width_a, &[(0, 0), (1000, 0)])
            + &wire("F.Cu", width_b, &[(0, gap), (1000, gap)])
    };
    // Two legs 0.1 mm apart at width 0.2 mm.
    assert_eq!(kinds_of(legs(200, 200, 100)), [ContactKind::Touch]);
    // Half the widths minus 2 µm is a touch, minus 0 µm is not; minus 1 µm, the
    // margin itself, is not either.
    assert_eq!(kinds_of(legs(200, 200, 198)), [ContactKind::Touch]);
    assert!(kinds_of(legs(200, 200, 199)).is_empty());
    assert!(kinds_of(legs(200, 200, 200)).is_empty());
    // Unequal widths: half their sum, 150 µm.
    assert_eq!(kinds_of(legs(200, 100, 148)), [ContactKind::Touch]);
    assert!(kinds_of(legs(200, 100, 149)).is_empty());
    assert!(kinds_of(legs(200, 100, 150)).is_empty());
    // The same gap is a touch for wide copper and none for narrow.
    assert_eq!(kinds_of(legs(400, 400, 300)), [ContactKind::Touch]);
    assert!(kinds_of(legs(100, 100, 300)).is_empty());
    // An end close to the other piece's side, not on it.
    assert_eq!(
        kinds_of(
            wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(500, 150), (500, 900)])
        ),
        [ContactKind::Touch]
    );
    assert!(kinds_of(
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(500, 250), (500, 900)])
    )
    .is_empty());
    // A diagonal piece with the other's end off its side (the distance is across the
    // diagonal): 250 / sqrt 2 = 177 µm is a touch, 300 / sqrt 2 = 212 µm is not.
    let rising = |end_x: i64| {
        wire("F.Cu", 200, &[(0, 0), (1000, 1000)])
            + &wire("F.Cu", 200, &[(end_x, 300), (2000, 300)])
    };
    assert_eq!(kinds_of(rising(550)), [ContactKind::Touch]);
    assert!(kinds_of(rising(600)).is_empty());
    let falling = |end_x: i64| {
        wire("F.Cu", 200, &[(0, 1000), (1000, 0)])
            + &wire("F.Cu", 200, &[(end_x, 700), (2000, 700)])
    };
    assert_eq!(kinds_of(falling(550)), [ContactKind::Touch]);
    assert!(kinds_of(falling(600)).is_empty());
    // The other way round: the diagonal's end near a straight piece's side.
    assert_eq!(
        kinds_of(
            wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(500, 300), (700, 500)])
        ),
        [] as [ContactKind; 0]
    );
    assert_eq!(
        kinds_of(
            wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(500, 150), (700, 350)])
        ),
        [ContactKind::Touch]
    );
    // A touch on another layer is none.
    assert!(kinds_of(
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("B.Cu", 200, &[(0, 100), (1000, 100)])
    )
    .is_empty());
}

#[test]
fn pieces_are_too_far_apart_only_beyond_the_reach_of_their_widths_on_an_axis() {
    // Widths 200 and 200 µm at 10000 ticks per µm: the reach is 201 µm (half the
    // sum, 200, plus the 1 µm margin), 2 010 000 ticks.
    let piece = |width_um: i64, a: [i128; 2], b: [i128; 2]| Piece { width_um, a, b };
    let um = |x: i128| x * 10_000;
    let first = piece(200, [0, 0], [um(1000), 0]);
    let right = |gap_ticks: i128| piece(200, [um(1000) + gap_ticks, 0], [um(2000) + gap_ticks, 0]);
    let reach = um(201);
    assert!(!too_far_apart(&first, &right(0), 10_000.0));
    assert!(
        !too_far_apart(&first, &right(um(200)), 10_000.0),
        "200 µm is within reach"
    );
    assert!(
        !too_far_apart(&first, &right(reach), 10_000.0),
        "exactly the reach is not beyond it"
    );
    assert!(too_far_apart(&first, &right(reach + 1), 10_000.0));
    assert!(too_far_apart(&first, &right(um(5000)), 10_000.0));
    // Either order of the two pieces.
    assert!(too_far_apart(&right(reach + 1), &first, 10_000.0));
    assert!(!too_far_apart(&right(reach), &first, 10_000.0));
    // The other axis, in both orders.
    let above = |gap_ticks: i128| piece(200, [0, um(1000) + gap_ticks], [0, um(2000) + gap_ticks]);
    let tall = piece(200, [0, 0], [0, um(1000)]);
    assert!(!too_far_apart(&tall, &above(reach), 10_000.0));
    assert!(too_far_apart(&tall, &above(reach + 1), 10_000.0));
    assert!(too_far_apart(&above(reach + 1), &tall, 10_000.0));
    assert!(!too_far_apart(&above(reach), &tall, 10_000.0));
    // Apart on one axis only is enough; near on both is not apart.
    let corner = piece(200, [um(1000) + reach + 1, um(1000)], [um(2000), um(1000)]);
    assert!(too_far_apart(&first, &corner, 10_000.0));
    let near = piece(200, [um(1000), um(100)], [um(2000), um(100)]);
    assert!(!too_far_apart(&first, &near, 10_000.0));
    // The reach follows the widths: half their sum plus the margin.
    let narrow = piece(100, [0, 0], [um(1000), 0]);
    let narrow_right = |gap: i128| piece(100, [um(1000) + gap, 0], [um(2000) + gap, 0]);
    assert!(!too_far_apart(&narrow, &narrow_right(um(101)), 10_000.0));
    assert!(too_far_apart(&narrow, &narrow_right(um(101) + 1), 10_000.0));
    let mixed = piece(400, [um(1000) + um(251), 0], [um(2000), 0]);
    assert!(
        !too_far_apart(&first, &mixed, 10_000.0),
        "(200 + 400) / 2 + 1 = 301"
    );
    let mixed_far = piece(400, [um(1000) + um(302), 0], [um(2000), 0]);
    assert!(too_far_apart(&first, &mixed_far, 10_000.0));
    // The ticks per µm scale the reach.
    assert!(!too_far_apart(&first, &right(um(201) / 10), 1_000.0));
    assert!(too_far_apart(&first, &right(um(201) / 10 + 1), 1_000.0));
}

#[test]
fn a_session_that_does_not_parse_has_no_contacts_to_give() {
    assert!(self_contacts("not a session", &names(&["A"])).is_err());
    assert!(self_contacts("(session x)", &names(&["A"])).is_err());
}

// ---- Row LT, L21: contacts the Tune pass made ----------------------------------

fn new_contacts(before: &str, after: &str, nets: &[&str]) -> BTreeMap<String, Vec<SelfContact>> {
    new_self_contacts(before, after, &names(nets)).unwrap()
}

#[test]
fn a_contact_with_a_new_piece_names_the_net() {
    let held = session(&[("A", wire("F.Cu", 200, &[(0, 0), (1000, 0)]))]);
    let tuned = session(&[(
        "A",
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(500, -500), (500, 500)]),
    )]);
    let found = new_contacts(&held, &tuned, &["A"]);
    assert_eq!(found.keys().collect::<Vec<_>>(), ["A"]);
    assert_eq!(found["A"].len(), 1);
    assert_eq!(found["A"][0].kind, ContactKind::Crossing);
    // A net outside the asked nets is not looked at.
    assert!(new_contacts(&held, &tuned, &["B"]).is_empty());
    assert!(new_contacts(&held, &tuned, &[]).is_empty());
}

#[test]
fn a_contact_the_held_session_already_has_names_nothing() {
    let crossing =
        || wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(500, -500), (500, 500)]);
    let held = session(&[("A", crossing())]);
    // The same two pieces, written as the Tune pass might (another split, the other
    // direction): still the same raw pieces to 1 µm.
    let tuned = session(&[(
        "A",
        wire("F.Cu", 200, &[(500, 500), (500, -500)]) + &wire("F.Cu", 200, &[(1000, 0), (0, 0)]),
    )]);
    assert!(
        !contacts_of(crossing()).is_empty(),
        "the held contact exists"
    );
    assert!(new_contacts(&held, &tuned, &["A"]).is_empty());
    assert!(new_contacts(&held, &held, &["A"]).is_empty());
    // A held contact whose piece was replaced by a new contacting piece names the
    // net: the counts of contacts are the same, the identity is not.
    let replaced = session(&[(
        "A",
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(600, -500), (600, 500)]),
    )]);
    assert_eq!(new_contacts(&held, &replaced, &["A"]).len(), 1);
    // Rounding to 1 µm: a piece moved by 0.4 µm is the same piece, by 0.6 µm is not.
    let nudged = |dx_tenths: i64| {
        session_with(
            "um 10",
            &[(
                "A",
                format!(
                    "(wire (path F.Cu 2000 {} {} {} {})) (wire (path F.Cu 2000 5000 -5000 5000 5000))",
                    0,
                    0,
                    10000 + dx_tenths,
                    0
                ),
            )],
        )
    };
    let base = nudged(0);
    assert!(new_contacts(&base, &nudged(4), &["A"]).is_empty());
    assert_eq!(new_contacts(&base, &nudged(6), &["A"]).len(), 1);
}

#[test]
fn a_copy_of_a_held_piece_is_the_same_piece_and_a_distinct_overlapping_piece_is_new() {
    let once = session(&[("A", wire("F.Cu", 200, &[(0, 0), (1000, 0)]))]);
    let copies = |n: usize| session(&[("A", wire("F.Cu", 200, &[(0, 0), (1000, 0)]).repeat(n))]);
    // fastroute's load-time copies of a held piece are no new piece and no contact.
    for n in [2, 3, 4] {
        assert!(
            new_contacts(&once, &copies(n), &["A"]).is_empty(),
            "{n} copies"
        );
        assert!(new_contacts(&copies(n), &once, &["A"]).is_empty());
        assert!(new_contacts(&copies(2), &copies(n), &["A"]).is_empty());
    }
    // Control: a distinct piece laid over the held one is new, and names the net.
    let over = session(&[(
        "A",
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]) + &wire("F.Cu", 200, &[(500, 0), (1500, 0)]),
    )]);
    let found = new_contacts(&once, &over, &["A"]);
    assert_eq!(found["A"].len(), 1);
    assert_eq!(found["A"][0].kind, ContactKind::Overlap);
    // ...also when the held piece was copied in the Tune session.
    let over_copied = session(&[(
        "A",
        wire("F.Cu", 200, &[(0, 0), (1000, 0)]).repeat(3)
            + &wire("F.Cu", 200, &[(500, 0), (1500, 0)]),
    )]);
    assert_eq!(new_contacts(&once, &over_copied, &["A"])["A"].len(), 1);
}

#[test]
fn the_routers_own_s_bend_in_the_held_session_names_nothing() {
    // /DQ0 of A-tuned's held session: a jog that comes closer than a width.
    let own = self_contacts(A_DQ0_HELD, &names(&["/DQ0"])).unwrap();
    assert_eq!(own["/DQ0"].len(), 1);
    assert_eq!(own["/DQ0"][0].kind, ContactKind::Touch);
    assert!(new_contacts(A_DQ0_HELD, A_DQ0_HELD, &["/DQ0"]).is_empty());
    // After the Tune pass the touch is still there, but not new; the pass's other
    // change to /DQ0 is copies of held pieces, which are no new piece either.
    assert!(self_contacts(A_DQ0_TUNED, &names(&["/DQ0"]))
        .unwrap()
        .contains_key("/DQ0"));
    assert!(new_contacts(A_DQ0_HELD, A_DQ0_TUNED, &["/DQ0"]).is_empty());
}

/// The held C_N of B with its 20 µm stub (In1.Cu, inside a via's own copper at
/// (104.4926, -50.0396) mm) taken out, as the Tune pass with the router off leaves it.
fn without_the_stub(text: &str) -> String {
    let stub = "        (wire\n          (path In1.Cu 1270\n            1044926 -500396\n            1044926 -500596\n          )\n        )\n";
    assert!(text.contains(stub), "the stub is in the held session");
    text.replacen(stub, "", 1)
}

#[test]
fn a_held_stub_inside_a_via_that_the_tune_session_lacks_is_no_contact_and_no_fault() {
    let after = without_the_stub(B_C_N_HELD);
    assert!(B_C_N_HELD.contains("1044926 -500396") && !after.contains("1044926 -500396"));
    // The stub touches nothing in the held session, and its loss adds nothing.
    let held_own = self_contacts(B_C_N_HELD, &names(&[B_C_N])).unwrap();
    let after_own = self_contacts(&after, &names(&[B_C_N])).unwrap();
    assert_eq!(held_own[B_C_N], after_own[B_C_N]);
    assert!(new_contacts(B_C_N_HELD, &after, &[B_C_N]).is_empty());
}

#[test]
fn copies_of_a_held_bridge_do_not_name_c_n_but_two_new_crossings_name_d1_p() {
    // C_N of B's step 0b run: its 0.212 mm piece on In1.Cu is in the Tune session in
    // four copies (fastroute copies bridges when it loads a session). Nothing else
    // of C_N's contacts is new: it is not named.
    assert!(new_contacts(B_C_N_HELD, B_C_N_TUNED, &[B_C_N]).is_empty());
    // Control: D1_P, from the same session, got 2 crossings the held session does
    // not have.
    let d1_p = "/CM5/DPHY0_P.D1_P";
    let found = new_contacts(B_D1_P_HELD, B_D1_P_TUNED, &[d1_p]);
    let crossings: Vec<&SelfContact> = found[d1_p]
        .iter()
        .filter(|c| c.kind == ContactKind::Crossing)
        .collect();
    assert_eq!(crossings.len(), 2, "{:?}", found[d1_p]);
    assert!(crossings.iter().all(|c| c.layer == "B.Cu"));
    assert!(self_contacts(B_D1_P_HELD, &names(&[d1_p]))
        .unwrap()
        .values()
        .flatten()
        .all(|c| c.kind == ContactKind::Touch));
}

#[test]
fn sessions_that_do_not_parse_are_an_error_naming_which() {
    let good = session(&[("A", wire("F.Cu", 200, &[(0, 0), (1000, 0)]))]);
    let error = new_self_contacts("not a session", &good, &names(&["A"])).unwrap_err();
    assert!(format!("{error:#}").contains("before"), "{error:#}");
    let error = new_self_contacts(&good, "not a session", &names(&["A"])).unwrap_err();
    assert!(format!("{error:#}").contains("after"), "{error:#}");
}

#[test]
fn a_qarc_is_skipped_and_the_straight_pieces_are_still_checked() {
    // An arc is not a straight piece: it is no contact and no error (the importer accepts
    // it), and the net's straight pieces are checked as before.
    let arc = "(wire (qarc F.Cu 2000 0 0 5000 5000 5000 0))".to_string();
    let cross =
        wire("F.Cu", 200, &[(0, 0), (1000, 1000)]) + &wire("F.Cu", 200, &[(0, 1000), (1000, 0)]);
    let found = self_contacts(&session(&[("A", arc.clone() + &cross)]), &names(&["A"])).unwrap();
    assert!(found["A"].iter().any(|c| c.kind == ContactKind::Crossing));
    assert!(
        self_contacts(&session(&[("A", arc.clone())]), &names(&["A"]))
            .unwrap()
            .is_empty()
    );
    // Control: the straight pieces alone give the same contacts.
    assert_eq!(
        self_contacts(&session(&[("A", cross)]), &names(&["A"])).unwrap(),
        found
    );
    // Control: another shape that is not a path is still an error.
    let polygon = "(wire (polygon F.Cu 0 0 0 10 10 10))".to_string();
    assert!(self_contacts(&session(&[("A", polygon)]), &names(&["A"])).is_err());
}

// ---- on a system that cannot isolate a run ---------------------------------

#[test]
fn another_pass_needs_a_deadline_at_least_twice_the_last_pass_away() {
    let left = |seconds: f64| Duration::from_secs_f64(seconds);
    assert!(
        room_for_another_pass(left(2.0), 1.0),
        "exactly twice is enough"
    );
    assert!(room_for_another_pass(left(2.5), 1.0));
    assert!(room_for_another_pass(left(100.0), 10.0));
    assert!(!room_for_another_pass(left(1.99), 1.0));
    assert!(!room_for_another_pass(left(0.6), 1.0));
    assert!(!room_for_another_pass(left(0.0), 1.0));
    // A pass of no time needs no time.
    assert!(room_for_another_pass(left(0.0), 0.0));
}

#[tokio::test]
async fn the_deadline_helper_reads_the_time_left_and_never_goes_below_zero() {
    let now = Instant::now();
    assert!(deadline_leaves_room(now + Duration::from_secs(60), 1.0));
    assert!(!deadline_leaves_room(now + Duration::from_secs(60), 40.0));
    assert!(!deadline_leaves_room(now - Duration::from_millis(1), 1.0));
    assert!(deadline_leaves_room(now - Duration::from_millis(1), 0.0));
}

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
    // The folder the session is written in (`tune`, `tune-r2`, ...): a pass can be
    // told apart from the same pass in another round.
    char folder[128];
    {
        const char *fb = strrchr(sesdir, '/');
        snprintf(folder, sizeof folder, "%s", fb ? fb + 1 : sesdir);
    }
    snprintf(key, sizeof key, "sleep.%s", pass);
    if (cfg(key, val, sizeof val)) {
        sleep(30);
        return 0;
    }
    // `delay_ms.<folder>=N` or `delay_ms=N`: the pass takes that long, then goes on.
    snprintf(key, sizeof key, "delay_ms.%s", folder);
    {
        int delay = cfg_int(key, cfg_int("delay_ms", 0));
        if (delay > 0) usleep((useconds_t)delay * 1000);
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
        snprintf(key, sizeof key, "copy_session.%s", folder);
        int have = cfg(key, src, sizeof src);
        if (!have) {
            snprintf(key, sizeof key, "copy_session.%s", pass);
            have = cfg(key, src, sizeof src) || cfg("copy_session", src, sizeof src);
        }
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

    /// Row LT, L6 and L7, through the production runner: with the call's Tune inputs
    /// every run ends with the Tune pass, which gets `--tune`, every class ignored
    /// and its own layer heights, and never `--pairs`; the routing passes get the
    /// routing heights and the pairs file, and never `--tune`. A runner without Tune
    /// inputs and heights runs the routing pass alone, as before.
    #[tokio::test]
    async fn every_run_of_a_search_tunes_with_the_calls_tune_inputs() {
        use crate::route_judge::JudgeContext;
        use crate::route_search::{Runner, Shape};
        use crate::router::ToolRouter;
        use crate::tools::route_fastroute::{FastrouteRunner, TuneFiles};
        use crate::tools::{ServerConfig, ToolContext};
        use std::sync::Arc;

        let fixture = Fixture::new("unrouted.full=0\n");
        let search = fixture.path().join("search");
        std::fs::create_dir_all(&search).unwrap();
        let pairs = search.join("pairs.txt");
        std::fs::write(&pairs, "pair /CAN_H /CAN_L skew=0.1\n").unwrap();
        let tune_file = search.join("tune.txt");
        std::fs::write(&tune_file, "group lt1 tolerance=0.25\n  /CAN_H\n  /CAN_L\n").unwrap();
        let runner = |routing_heights: Option<&str>, tune: Option<TuneFiles>| FastrouteRunner {
            binary: fixture.binary(),
            dsn_text: Arc::new(RUNNER_DSN.to_string()),
            hints: floor_hints(),
            amps: None,
            pairs: Some(pairs.clone()),
            routing_heights: routing_heights.map(str::to_string),
            tune,
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
        let args_of = |folder: &Path, name: &str| -> Vec<String> {
            std::fs::read_to_string(folder.join(name).join(format!("{name}.ses.args")))
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect()
        };
        let tune = TuneFiles {
            file: tune_file.clone(),
            classes: vec!["c1".to_string(), "c2".to_string()],
            nets: ["/CAN_H", "/CAN_L"].iter().map(|n| n.to_string()).collect(),
            layer_heights: Some("0,1.6".to_string()),
            plan: std::sync::Arc::new(crate::tools::route_lengths::LengthPlan::default()),
        };
        let with = runner(Some("0,1.58"), Some(tune));
        for (index, shape, seed) in [(0, Shape::AsExported, 1), (1, Shape::Omitted, 1)] {
            let routed = with.route(index, shape, seed, soon(60)).await.unwrap();
            let names: Vec<&str> = routed
                .held
                .passes
                .iter()
                .map(|row| row["name"].as_str().unwrap())
                .collect();
            assert_eq!(names, ["full", "fallback", "polish", "tune"], "run {index}");
            let full = args_of(&routed.folder, "full");
            assert!(
                full.contains(&"--layer-heights=0,1.58".to_string()),
                "{full:?}"
            );
            assert!(
                full.contains(&format!("--pairs={}", pairs.display())),
                "{full:?}"
            );
            assert!(
                !full.iter().any(|arg| arg.starts_with("--tune")),
                "{full:?}"
            );
            let tuned = args_of(&routed.folder, "tune");
            assert!(
                tuned.contains(&format!("--tune={}", tune_file.display())),
                "{tuned:?}"
            );
            assert!(
                tuned.contains(&"--layer-heights=0,1.6".to_string()),
                "{tuned:?}"
            );
            assert!(
                tuned.contains(&"--router.autorouter.ignore_net_classes=c1,c2".to_string()),
                "{tuned:?}"
            );
            assert!(
                !tuned.iter().any(|arg| arg.starts_with("--pairs")),
                "{tuned:?}"
            );
        }
        // Control: no Tune inputs and no routing heights, the call of a board
        // without groups or pairs: three routing rows, no `--tune`, no heights.
        let without = runner(None, None);
        let routed = without
            .route(10, Shape::AsExported, 1, soon(60))
            .await
            .unwrap();
        assert_eq!(routed.held.passes.len(), 3, "{:?}", routed.held.passes);
        assert!(!routed.folder.join("tune").exists());
        let full = args_of(&routed.folder, "full");
        assert!(
            !full
                .iter()
                .any(|arg| arg.starts_with("--tune") || arg.starts_with("--layer-heights")),
            "{full:?}"
        );
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
        // What each case is called, the design intent, the edit of the DSN and of the
        // board, and a word the refusal must name.
        type Edit = Box<dyn Fn(&str) -> String>;
        let cases: Vec<(&str, String, Edit, Edit, &str)> = vec![
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
                "a DSN whose net classes cannot be read",
                group_intent(),
                Box::new(rename(
                    "(class konnect_class_0001 GND S1",
                    "(class (x) GND S1",
                )),
                Box::new(unchanged),
                "net classes could not be read",
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
            Some(tune_request(tune_inputs(
                &tune_file,
                &classes,
                &nets,
                Some("0,1.58"),
            ))),
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
        assert_eq!(tune_row(&held), ("tune", "candidate", None));
        // The candidate is kept beside the untuned session, which the run still holds
        // until the judge chooses (L23).
        assert_eq!(held.kept, "full");
        assert_eq!(held.ses, fixture.path().join("held/full/full.ses"));
        assert_eq!(held.unrouted, 0);
        let candidate = &held.tune.candidates[0];
        assert_eq!(candidate.ses, fixture.path().join("held/tune/tune.ses"));
        assert_eq!(candidate.unrouted, 0);
        assert_eq!(
            std::fs::read_to_string(&candidate.ses).unwrap(),
            tuned_base()
        );
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
        assert_eq!(tune_row(&held.unwrap()).1, "candidate");
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
            assert_eq!(tune_row(&held), ("tune", "candidate", None));
            // The candidate holds its count; the run's own stays the untuned one.
            assert_eq!(held.tune.candidates[0].unrouted, expected);
            assert_eq!((held.kept, held.unrouted), ("full", 1));
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
        tune: Option<TuneRequest<'_>>,
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
            Some(tune_request(tune_inputs(
                &tune_file,
                &classes,
                &nets,
                Some("0,1.58"),
            ))),
        )
        .await;
        assert_eq!(held.tune.candidates.len(), 1, "{:?}", held.passes);
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
            Some(tune_request(tune_inputs(
                &tune_file,
                &classes,
                &nets,
                Some("0,1.58"),
            ))),
        )
        .await;
        assert_eq!(held.tune.candidates.len(), 1);
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
                Some(tune_request(tune_inputs(
                    &tune_file,
                    &classes,
                    &nets,
                    Some("0,1.58"),
                ))),
                soon(300),
            )
            .await
            .unwrap();
            let rows: Vec<_> = held.passes.iter().map(|r| r["outcome"].clone()).collect();
            let tune_row = held.passes.last().unwrap();
            assert_eq!(tune_row["name"], "tune", "{rows:?}");
            assert_eq!(tune_row["outcome"], "candidate", "{tune_row}");
            assert_eq!(held.tune.candidates.len(), 1);
            let full = dir.path().join(name).join("full/full.ses");
            assert!(changed_outside(
                &std::fs::read_to_string(&full).unwrap(),
                &std::fs::read_to_string(&held.tune.candidates[0].ses).unwrap(),
                &nets,
                &NO_VIA_RADII
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

    // ---- Row LT, L21 and L24: rounds, candidates ------------------------------

    /// A session of the nets `names`, each one 1 mm piece on F.Cu 5 mm below the
    /// last; a net of `crossed` also gets a piece across its middle, a new piece that
    /// crosses the net's own copper.
    fn rows_session(names: &[&str], crossed: &[&str]) -> String {
        let nets: Vec<(&str, String)> = names
            .iter()
            .enumerate()
            .map(|(row, name)| {
                let y = row as i64 * 5000;
                let mut items = wire("F.Cu", 200, &[(0, y), (1000, y)]);
                if crossed.contains(name) {
                    items.push_str(&wire("F.Cu", 200, &[(500, y - 400), (500, y + 400)]));
                }
                (*name, items)
            })
            .collect();
        session(&nets)
    }

    /// What the next round's tune file does in a test.
    enum NextMode<'a> {
        /// The nets of the call minus the dropped ones, written as a note; none left:
        /// no file.
        Without,
        /// `route_lengths::write_file_without` of this plan.
        Plan(&'a crate::tools::route_lengths::LengthPlan),
        /// An error.
        Fails,
    }

    struct Attempt {
        fixture: Fixture,
        result: std::result::Result<Option<TuneCandidate>, PassError>,
        outcome: TuneOutcome,
        /// The dropped set each call of the next-file function was given.
        calls: Vec<BTreeSet<String>>,
    }

    impl Attempt {
        fn args(&self, folder: &str) -> Vec<String> {
            std::fs::read_to_string(
                self.fixture
                    .path()
                    .join("run")
                    .join(folder)
                    .join("tune.ses.args"),
            )
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
        }
        fn folder(&self, name: &str) -> PathBuf {
            self.fixture.path().join("run").join(name)
        }
        fn reasons(&self) -> Vec<(String, String)> {
            self.outcome
                .rows
                .iter()
                .map(|row| {
                    (
                        row["outcome"].as_str().unwrap().to_string(),
                        row["reason"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        }
    }

    /// One candidate's [`tune_attempt`] through the fake, from the held session
    /// `held`: `rounds` gives the session each pass folder (`tune`, `tune-r2`, ...)
    /// writes.
    #[allow(clippy::too_many_arguments)]
    async fn attempt(
        variant: TuneVariant,
        held: &str,
        rounds: &[(&str, String)],
        control: &str,
        nets: &[&str],
        mode: NextMode<'_>,
        deadline: Instant,
    ) -> Attempt {
        attempt_at(
            variant,
            held,
            rounds,
            control,
            nets,
            mode,
            deadline,
            &Instant::now,
            &NO_VIA_RADII,
        )
        .await
    }

    /// [`attempt`] with the time the room rule reads fixed by `clock`.
    #[allow(clippy::too_many_arguments)] // the parts of one attempt, each read once
    async fn attempt_at(
        variant: TuneVariant,
        held: &str,
        rounds: &[(&str, String)],
        control: &str,
        nets: &[&str],
        mode: NextMode<'_>,
        deadline: Instant,
        clock: Clock<'_>,
        via_radii: &BTreeMap<String, i64>,
    ) -> Attempt {
        let fixture = Fixture::new("");
        let held_path = fixture.path().join("held.ses");
        std::fs::write(&held_path, held).unwrap();
        let mut lines = String::new();
        for (folder, text) in rounds {
            let path = fixture.path().join(format!("{folder}.session"));
            std::fs::write(&path, text).unwrap();
            lines.push_str(&format!("copy_session.{folder}={}\n", path.display()));
        }
        fixture.set_control(&format!("{lines}{control}"));
        let tune_file = fixture.path().join("tune.txt");
        std::fs::write(&tune_file, "group lt1 tolerance=0.5\n  X\n").unwrap();
        let pairs = fixture.path().join("pairs.txt");
        std::fs::write(&pairs, "pair /X /Y gap=0.2\n").unwrap();
        let classes = class_list(&["c1", "c2"]);
        let nets = names(nets);
        let first = TuneInputs {
            via_radii,
            ..tune_inputs(&tune_file, &classes, &nets, Some("0,1.58"))
        };
        let calls = std::sync::Mutex::new(Vec::new());
        let next = |folder: &Path,
                    dropped: &BTreeSet<String>|
         -> Result<Option<(PathBuf, BTreeSet<String>)>> {
            calls.lock().unwrap().push(dropped.clone());
            match &mode {
                NextMode::Without => {
                    let listed: BTreeSet<String> = nets.difference(dropped).cloned().collect();
                    if listed.is_empty() {
                        return Ok(None);
                    }
                    let path = folder.join("tune.txt");
                    std::fs::write(&path, format!("# without {dropped:?}\n")).unwrap();
                    Ok(Some((path, listed)))
                }
                NextMode::Plan(plan) => {
                    crate::tools::route_lengths::write_file_without(folder, plan, dropped)
                }
                NextMode::Fails => bail!("a disk that is full"),
            }
        };
        let mut outcome = TuneOutcome::default();
        let result = tune_attempt(
            &fixture.binary(),
            &fixture.path().join("run"),
            variant,
            &fixture.dsn(),
            &floor_hints(),
            CallFiles {
                amps: None,
                pairs: Some(&pairs),
                layer_heights: Some("9,9"),
            },
            &first,
            &next,
            &held_path,
            0,
            deadline,
            clock,
            &mut outcome,
        )
        .await;
        let calls = calls.into_inner().unwrap();
        Attempt {
            fixture,
            result,
            outcome,
            calls,
        }
    }

    /// The candidate an attempt gave, if any.
    fn kept_of(run: &Attempt) -> Option<&TuneCandidate> {
        run.result
            .as_ref()
            .expect("the attempt did not fail")
            .as_ref()
    }

    const XYZ: [&str; 3] = ["X", "Y", "Z"];

    #[tokio::test]
    async fn a_pass_without_contacts_is_the_candidate_after_one_pass() {
        let held = rows_session(&XYZ, &[]);
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[("tune", rows_session(&XYZ, &[]))],
            "",
            &XYZ,
            NextMode::Without,
            soon(30),
        )
        .await;
        let candidate = kept_of(&run).expect("a candidate");
        assert_eq!(candidate.variant, TuneVariant::Free);
        assert_eq!(candidate.passes, 1);
        assert!(candidate.dropped.is_empty());
        assert_eq!(candidate.unrouted, 0);
        assert_eq!(candidate.ses, run.fixture.path().join("run/tune/tune.ses"));
        assert!(run.calls.is_empty(), "no next round was asked for");
        assert!(!run.folder("tune-r2").exists());
        assert_eq!(run.outcome.rows.len(), 1);
        let row = &run.outcome.rows[0];
        assert_eq!(
            (
                row["name"].clone(),
                row["outcome"].clone(),
                row["variant"].clone(),
                row["round"].clone()
            ),
            (json!("tune"), json!("candidate"), json!("free"), json!(1))
        );
        assert!(run.outcome.seconds > 0.0);
        // The pass gets its own heights and never the call's pairs file.
        let args = run.args("tune");
        assert!(!args.iter().any(|t| t.starts_with("--pairs")), "{args:?}");
        assert_eq!(value(&args, "--layer-heights="), Some("0,1.58"));
        assert!(args.contains(&TUNE_ROUTER_OFF.to_string()));
        assert!(!args.contains(&TUNE_POUR_SAFE.to_string()));
    }

    #[tokio::test]
    async fn a_net_that_touches_itself_is_dropped_and_the_next_round_runs_without_it() {
        let held = rows_session(&XYZ, &[]);
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[
                ("tune", rows_session(&XYZ, &["X"])),
                ("tune-r2", rows_session(&XYZ, &[])),
            ],
            "",
            &XYZ,
            NextMode::Without,
            soon(60),
        )
        .await;
        let candidate = kept_of(&run).expect("a candidate");
        assert_eq!(candidate.passes, 2);
        assert_eq!(
            candidate.ses,
            run.fixture.path().join("run/tune-r2/tune.ses")
        );
        assert_eq!(candidate.dropped.len(), 1);
        assert_eq!(candidate.dropped[0].0, "X");
        assert!(
            candidate.dropped[0].1.contains("1 crossing"),
            "{:?}",
            candidate.dropped
        );
        assert_eq!(run.calls, vec![names(&["X"])]);
        let reasons = run.reasons();
        assert_eq!(reasons.len(), 2);
        assert_eq!(reasons[0].0, "discarded");
        assert!(
            reasons[0].1.starts_with("meanders touch their own net: X"),
            "{reasons:?}"
        );
        assert!(reasons[0].1.contains("tuned again without"), "{reasons:?}");
        assert_eq!(reasons[1], ("candidate".to_string(), String::new()));
        assert_eq!(run.outcome.rows[1]["round"], json!(2));
        assert_eq!(run.outcome.rows[1]["variant"], json!("free"));
        // Round 2 is given its own tune file and the same held session.
        let round_two = run.args("tune-r2");
        let held_path = run.fixture.path().join("held.ses");
        assert_eq!(
            value(&round_two, "--tune="),
            Some(run.folder("tune-r2").join("tune.txt").to_str().unwrap())
        );
        assert_eq!(
            value(&round_two, "--initial-session="),
            Some(held_path.to_str().unwrap())
        );
        assert!(
            std::fs::read_to_string(run.folder("tune-r2").join("tune.txt"))
                .unwrap()
                .contains("\"X\"")
        );
        assert_eq!(
            value(&run.args("tune"), "--tune="),
            Some(run.fixture.path().join("tune.txt").to_str().unwrap())
        );
        assert!(!round_two.iter().any(|t| t.starts_with("--pairs")));
        assert_eq!(value(&round_two, "--layer-heights="), Some("0,1.58"));
        // The seconds of both passes are counted.
        let total: f64 = run
            .outcome
            .rows
            .iter()
            .map(|row| row["seconds"].as_f64().unwrap())
            .sum();
        assert!((run.outcome.seconds - total).abs() < 1e-9);
    }

    #[tokio::test]
    async fn contacts_on_every_pass_end_in_a_discard_after_three_rounds() {
        let held = rows_session(&XYZ, &[]);
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[
                ("tune", rows_session(&XYZ, &["X"])),
                ("tune-r2", rows_session(&XYZ, &["Y"])),
                ("tune-r3", rows_session(&XYZ, &["Z"])),
                ("tune-r4", rows_session(&XYZ, &[])),
            ],
            "",
            &XYZ,
            NextMode::Without,
            soon(120),
        )
        .await;
        assert!(kept_of(&run).is_none());
        assert_eq!(TUNE_ROUNDS_MAX, 3);
        assert_eq!(run.calls, vec![names(&["X"]), names(&["X", "Y"])]);
        let reasons = run.reasons();
        assert_eq!(reasons.len(), 3, "{reasons:?}");
        assert!(reasons.iter().all(|(outcome, _)| outcome == "discarded"));
        assert!(reasons[0].1.starts_with("meanders touch their own net: X"));
        assert!(reasons[1].1.starts_with("meanders touch their own net: Y"));
        assert_eq!(
            reasons[2].1,
            "meanders touch their own net: Z (no round left)"
        );
        assert!(run.folder("tune-r3").is_dir());
        assert!(!run.folder("tune-r4").exists(), "no fourth pass");
        assert_eq!(run.outcome.rows[2]["round"], json!(3));
    }

    #[tokio::test]
    async fn a_pass_is_discarded_when_no_group_is_left_or_the_next_file_cannot_be_written() {
        let held = rows_session(&XYZ, &[]);
        // The only tuned net touches itself: `next` has no group to write.
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[("tune", rows_session(&XYZ, &["X"]))],
            "",
            &["X"],
            NextMode::Without,
            soon(60),
        )
        .await;
        assert!(kept_of(&run).is_none());
        assert_eq!(run.calls.len(), 1);
        assert_eq!(
            run.reasons(),
            [(
                "discarded".to_string(),
                "meanders touch their own net: X".to_string()
            )]
        );
        assert!(!run.folder("tune-r2/tune.ses").exists());
        // A file that cannot be written.
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[("tune", rows_session(&XYZ, &["X"]))],
            "",
            &XYZ,
            NextMode::Fails,
            soon(60),
        )
        .await;
        assert!(kept_of(&run).is_none());
        let reasons = run.reasons();
        assert_eq!(reasons.len(), 1);
        assert!(
            reasons[0].1.starts_with("meanders touch their own net: X"),
            "{reasons:?}"
        );
        assert!(reasons[0].1.contains("a disk that is full"), "{reasons:?}");
    }

    #[tokio::test]
    async fn a_round_is_not_started_when_the_deadline_leaves_less_than_twice_the_last_pass() {
        let held = rows_session(&XYZ, &[]);
        let sessions = [
            ("tune", rows_session(&XYZ, &["X"])),
            ("tune-r2", rows_session(&XYZ, &[])),
        ];
        // The deadline is far away (so no pass is ever killed), and the clock the room
        // rule reads stands at the deadline: nothing is left, which is less than twice any
        // pass, whatever the machine's load.
        let deadline = soon(3600);
        let at_deadline = move || deadline;
        let run = attempt_at(
            TuneVariant::Free,
            &held,
            &sessions,
            "",
            &XYZ,
            NextMode::Without,
            deadline,
            &at_deadline,
            &NO_VIA_RADII,
        )
        .await;
        assert!(kept_of(&run).is_none());
        assert!(run.calls.is_empty(), "no next file was written");
        assert_eq!(
            run.reasons(),
            [(
                "discarded".to_string(),
                "meanders touch their own net: X (the deadline leaves no room)".to_string()
            )]
        );
        assert!(!run.folder("tune-r2").exists());
        // Control: the real clock with the same deadline leaves far more than twice the
        // pass, so the second round runs and is kept.
        let run = attempt(
            TuneVariant::Free,
            &held,
            &sessions,
            "",
            &XYZ,
            NextMode::Without,
            deadline,
        )
        .await;
        assert_eq!(
            kept_of(&run).expect("a candidate").passes,
            2,
            "{:?}",
            run.outcome.rows
        );
    }

    #[tokio::test]
    async fn a_via_stub_the_load_dropped_does_not_discard_a_tune_pass() {
        // Net A is outside the tune file {X}; the pass writes it back without its 20 um
        // stub at a via, which is all that changed.
        let with_stub = |stub: bool| {
            session(&[
                ("X", wire("F.Cu", 200, &[(0, 0), (5000, 0)])),
                (
                    "A",
                    wire("F.Cu", 200, &[(0, 1000), (1000, 1000)])
                        + &via("v", 1000, 1000)
                        + &if stub {
                            wire("In1.Cu", 200, &[(1000, 1000), (1000, 1020)])
                        } else {
                            String::new()
                        },
                ),
            ])
        };
        let radii = BTreeMap::from([("v".to_string(), 225)]);
        let rounds = [("tune", with_stub(false))];
        let run = attempt_at(
            TuneVariant::Free,
            &with_stub(true),
            &rounds,
            "",
            &["X"],
            NextMode::Without,
            soon(3600),
            &Instant::now,
            &radii,
        )
        .await;
        assert!(kept_of(&run).is_some(), "{:?}", run.outcome.rows);
        // Control: without the via's disc the pass is discarded for net A, as before.
        let run = attempt(
            TuneVariant::Free,
            &with_stub(true),
            &rounds,
            "",
            &["X"],
            NextMode::Without,
            soon(3600),
        )
        .await;
        assert!(kept_of(&run).is_none());
        assert_eq!(
            run.reasons(),
            [(
                "discarded".to_string(),
                "changed the copper of A".to_string()
            )]
        );
    }

    /// A skew group of four nets, `A` and `B` a pair.
    fn paired_plan() -> crate::tools::route_lengths::LengthPlan {
        use crate::tools::route_lengths::{GroupNet, LengthPlan, ResolvedGroup, Window};
        LengthPlan {
            groups: vec![ResolvedGroup {
                interface: "i".to_string(),
                name: "g".to_string(),
                file_name: "lt1".to_string(),
                nets: ["A", "B", "C", "D"]
                    .iter()
                    .map(|net| GroupNet {
                        intent: net.to_string(),
                        dsn: net.to_string(),
                    })
                    .collect(),
                window: Window::Skew { tolerance_mm: 0.5 },
                pairs: vec![("A".to_string(), "B".to_string(), 0.025)],
            }],
            ..LengthPlan::default()
        }
    }

    #[tokio::test]
    async fn a_net_that_is_half_of_a_pair_takes_its_partner_and_the_pair_group_with_it() {
        const ABCD: [&str; 4] = ["A", "B", "C", "D"];
        let held = rows_session(&ABCD, &[]);
        let plan = paired_plan();
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[
                ("tune", rows_session(&ABCD, &["A"])),
                ("tune-r2", rows_session(&ABCD, &[])),
            ],
            "",
            &ABCD,
            NextMode::Plan(&plan),
            soon(60),
        )
        .await;
        let candidate = kept_of(&run).expect("a candidate");
        assert_eq!(candidate.passes, 2);
        let dropped: Vec<&str> = candidate
            .dropped
            .iter()
            .map(|(net, _)| net.as_str())
            .collect();
        assert_eq!(dropped, ["A", "B"]);
        assert!(
            candidate.dropped[0].1.contains("crossing"),
            "{:?}",
            candidate.dropped
        );
        assert!(
            candidate.dropped[1].1.contains("left out with"),
            "{:?}",
            candidate.dropped
        );
        let file = std::fs::read_to_string(run.folder("tune-r2").join("tune.txt")).unwrap();
        assert!(
            file.contains("group lt1 tolerance=0.25\n  C\n  D\n"),
            "{file}"
        );
        assert!(!file.contains("lt1p1") && !file.contains("  A\n") && !file.contains("  B\n"));
        // Control: a net with no partner takes only itself, and the pair group stays.
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[
                ("tune", rows_session(&ABCD, &["C"])),
                ("tune-r2", rows_session(&ABCD, &[])),
            ],
            "",
            &ABCD,
            NextMode::Plan(&plan),
            soon(60),
        )
        .await;
        let candidate = kept_of(&run).expect("a candidate");
        assert_eq!(candidate.dropped.len(), 1);
        assert_eq!(candidate.dropped[0].0, "C");
        let file = std::fs::read_to_string(run.folder("tune-r2").join("tune.txt")).unwrap();
        assert!(
            file.contains("group lt1 tolerance=0.25\n  A\n  B\n  D\n"),
            "{file}"
        );
        assert!(
            file.contains("group lt1p1 tolerance=0.025\n  A\n  B\n"),
            "{file}"
        );
    }

    #[tokio::test]
    async fn the_discard_reasons_of_a_pass_that_opened_or_changed_come_before_the_contacts() {
        let held = rows_session(&XYZ, &[]);
        // Changed another net: Y is not tuned here.
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[("tune", rows_session(&XYZ, &["Y"]))],
            "",
            &["X"],
            NextMode::Without,
            soon(60),
        )
        .await;
        assert!(kept_of(&run).is_none());
        assert_eq!(
            run.reasons(),
            [(
                "discarded".to_string(),
                "changed the copper of Y".to_string()
            )]
        );
        assert!(run.calls.is_empty());
        // Opened a connection: the held session left none open.
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[("tune", rows_session(&XYZ, &["X"]))],
            "unrouted.tune=2\n",
            &XYZ,
            NextMode::Without,
            soon(60),
        )
        .await;
        assert!(kept_of(&run).is_none());
        assert_eq!(
            run.reasons(),
            [("discarded".to_string(), "opened 2".to_string())]
        );
        assert!(run.calls.is_empty());
        // A pass that fails is a failed row and no candidate.
        let run = attempt(
            TuneVariant::Free,
            &held,
            &[("tune", rows_session(&XYZ, &[]))],
            "fail.tune=1\n",
            &XYZ,
            NextMode::Without,
            soon(60),
        )
        .await;
        assert!(kept_of(&run).is_none());
        assert_eq!(run.outcome.rows[0]["outcome"], json!("failed"));
        assert!(run.outcome.rows[0]["reason"]
            .as_str()
            .unwrap()
            .contains("fake failure marker for tune"));
    }

    #[tokio::test]
    async fn the_pour_safe_attempt_runs_in_its_own_folder_with_the_plane_switch() {
        let held = rows_session(&XYZ, &[]);
        let run = attempt(
            TuneVariant::PourSafe,
            &held,
            &[
                ("tune-pour-safe", rows_session(&XYZ, &["X"])),
                ("tune-pour-safe-r2", rows_session(&XYZ, &[])),
            ],
            "",
            &XYZ,
            NextMode::Without,
            soon(60),
        )
        .await;
        let candidate = kept_of(&run).expect("a candidate");
        assert_eq!(candidate.variant, TuneVariant::PourSafe);
        assert_eq!(
            candidate.ses,
            run.fixture.path().join("run/tune-pour-safe-r2/tune.ses")
        );
        assert!(!run.folder("tune").exists());
        for folder in ["tune-pour-safe", "tune-pour-safe-r2"] {
            assert!(
                run.args(folder).contains(&TUNE_POUR_SAFE.to_string()),
                "{folder}"
            );
        }
        assert_eq!(run.outcome.rows[0]["variant"], json!("pour_safe"));
    }

    /// The hold sequence with a Tune pass for a DSN whose planes are `planes`, the
    /// Free candidate writing `free` and the pour-safe one `pour`.
    async fn candidates_run(
        free: &str,
        pour: &str,
        planes: &[&str],
        control: &str,
        seconds: u64,
    ) -> (Fixture, Held) {
        let fixture = Fixture::new("");
        let write = |name: &str, text: &str| -> PathBuf {
            let path = fixture.path().join(name);
            std::fs::write(&path, text).unwrap();
            path
        };
        let (before, free, pour) = (
            write("before.ses", &base()),
            write("free.ses", free),
            write("pour.ses", pour),
        );
        fixture.set_control(&format!(
            "copy_session.full={}\ncopy_session.tune={}\ncopy_session.tune-pour-safe={}\n{control}",
            before.display(),
            free.display(),
            pour.display(),
        ));
        let tune_file = fixture.path().join("tune.txt");
        std::fs::write(&tune_file, "group lt1 tolerance=0.5\n  A\n").unwrap();
        let classes = class_list(&["c1"]);
        let nets = names(&["A"]);
        let planes: Vec<String> = planes.iter().map(|net| net.to_string()).collect();
        let held = route_held(
            &fixture.binary(),
            &fixture.path().join("held"),
            &fixture.dsn(),
            &floor_hints(),
            CallFiles::default(),
            WidthFallback::None,
            Some(TuneRequest {
                inputs: TuneInputs {
                    planes: &planes,
                    ..tune_inputs(&tune_file, &classes, &nets, None)
                },
                next: &no_next,
            }),
            soon(seconds),
        )
        .await
        .unwrap();
        (fixture, held)
    }

    fn moved_b() -> String {
        session(&[
            (
                "A",
                wire("F.Cu", 250, &[(0, 0), (10000, 0), (10000, 5000)]) + &via("v", 10000, 5000),
            ),
            ("B", wire("F.Cu", 250, &[(0, 1000), (20000, 1300)])),
            ("C", wire("B.Cu", 200, &[(0, 2000), (0, 8000)])),
        ])
    }

    #[tokio::test]
    async fn the_pour_safe_candidate_runs_only_when_the_dsn_has_a_plane_of_a_net_outside_the_tune_file(
    ) {
        for (planes, runs) in [
            (&[][..], false),
            (&["A"][..], false),
            (&["GND"][..], true),
            (&["A", "GND"][..], true),
        ] {
            let (fixture, held) =
                candidates_run(&tuned_base(), &tuned_base(), planes, "", 60).await;
            let folder = fixture.path().join("held/tune-pour-safe");
            assert_eq!(folder.is_dir(), runs, "{planes:?}");
            let variants: Vec<TuneVariant> =
                held.tune.candidates.iter().map(|c| c.variant).collect();
            assert_eq!(
                variants,
                if runs {
                    vec![TuneVariant::Free, TuneVariant::PourSafe]
                } else {
                    vec![TuneVariant::Free]
                },
                "{planes:?}"
            );
            let free_args = pass_args(&fixture, "tune");
            assert!(!free_args.contains(&TUNE_POUR_SAFE.to_string()));
            assert!(free_args.contains(&TUNE_ROUTER_OFF.to_string()));
            if runs {
                let args: Vec<String> = std::fs::read_to_string(folder.join("tune.ses.args"))
                    .unwrap()
                    .lines()
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    args.iter().filter(|t| *t == TUNE_POUR_SAFE).count(),
                    1,
                    "{planes:?}"
                );
            }
            // The run holds its untuned session until the judge chooses (L23): the
            // candidates are kept beside it, and `TuneChoice::apply` rewrites it.
            assert_eq!(held.kept, "full");
            assert_eq!(held.ses, fixture.path().join("held/full/full.ses"));
            assert_eq!(
                held.untuned,
                Some(fixture.path().join("held/full/full.ses"))
            );
            assert_eq!(
                held.tune.candidates[0].ses,
                fixture.path().join("held/tune/tune.ses")
            );
            assert_eq!(held.passes.len(), 4, "{:?}", held.passes);
            assert_eq!(held.tune.rows.len(), if runs { 2 } else { 1 });
            let seconds: f64 = held.passes[0]["seconds"].as_f64().unwrap()
                + held
                    .tune
                    .rows
                    .iter()
                    .map(|row| row["seconds"].as_f64().unwrap())
                    .sum::<f64>();
            assert!((held.seconds - seconds).abs() < 1e-9, "{}", held.seconds);
        }
    }

    #[tokio::test]
    async fn a_pour_safe_candidate_alone_does_not_replace_the_held_session() {
        // Free changes another net's copper and is discarded; the pour-safe pass is
        // fine. The judge chooses it or not (L23); the run keeps its session.
        let (fixture, held) = candidates_run(&moved_b(), &tuned_base(), &["GND"], "", 60).await;
        let variants: Vec<TuneVariant> = held.tune.candidates.iter().map(|c| c.variant).collect();
        assert_eq!(variants, [TuneVariant::PourSafe]);
        assert_eq!(held.kept, "full");
        assert_eq!(held.ses, fixture.path().join("held/full/full.ses"));
        assert_eq!(
            held.untuned,
            Some(fixture.path().join("held/full/full.ses"))
        );
        assert_eq!(
            tune_row(&held),
            ("tune", "discarded", Some("changed the copper of B"))
        );
        // Both discarded: no candidate, and no untuned copy to judge beside it.
        let (_fixture, held) = candidates_run(&moved_b(), &moved_b(), &["GND"], "", 60).await;
        assert!(held.tune.candidates.is_empty());
        assert_eq!(held.untuned, None);
        assert_eq!(held.kept, "full");
    }

    #[tokio::test]
    async fn the_pour_safe_attempt_is_skipped_when_the_deadline_leaves_less_than_twice_a_tune_pass()
    {
        // The Free pass takes 4 s or more; with 10 s in all, less than twice that is
        // left at any load, and the pass is not killed.
        let (fixture, held) = candidates_run(
            &tuned_base(),
            &tuned_base(),
            &["GND"],
            "delay_ms.tune=4000\n",
            10,
        )
        .await;
        assert!(!fixture.path().join("held/tune-pour-safe").exists());
        let skipped = held.tune.rows.last().unwrap();
        assert_eq!(skipped["outcome"], json!("skipped"));
        assert_eq!(skipped["variant"], json!("pour_safe"));
        assert!(skipped["reason"]
            .as_str()
            .unwrap()
            .contains("less than twice"));
        assert_eq!(held.tune.candidates.len(), 1);
        // The skipped row is not a pass run and adds no time.
        assert!(skipped["seconds"].is_null());
        // Control: a short pass leaves room for it.
        let (fixture, held) = candidates_run(
            &tuned_base(),
            &tuned_base(),
            &["GND"],
            "delay_ms.tune=100\n",
            60,
        )
        .await;
        assert!(fixture.path().join("held/tune-pour-safe").is_dir());
        assert_eq!(held.tune.candidates.len(), 2);
    }
}
