//! Row FF: the flow's model, without KiCad, fastroute or a clock.

use super::*;

fn facts() -> PlanFacts {
    PlanFacts {
        change: Some("placement".to_string()),
        unlocked_routing: 12,
        locked_routing: 0,
        locked_blocks_export: false,
        locked_on_moved: false,
        zones: 2,
        fill_clean: true,
        export_refused: None,
        named_only: Vec::new(),
        p4_on_winner: true,
    }
}

fn all_approved() -> Approvals {
    Approvals {
        placement: Some(PlacementAnswer::Apply),
        delete_routing: Some(DeleteAnswer::Delete),
        pours: Some(PoursAnswer::Add),
        pour_roles: Some(RolesAnswer::AsNamed(AsNamed::AsNamed)),
        routing: Some(RoutingAnswer::ApplyIfComplete),
    }
}

fn flow() -> FlowRecord {
    FlowRecord {
        flow_id: "1700000000000-0123456789ab".to_string(),
        board: "/b/board.kicad_pcb".to_string(),
        project: "/b/board.kicad_pro".to_string(),
        state: FlowState::Running,
        board_state: BoardState::Unchanged,
        steps_asked: Steps::default(),
        approvals: Approvals::default(),
        start_approvals: Approvals::default(),
        effort: "m".to_string(),
        placement_effort: "m".to_string(),
        fastroute: json!({}),
        planned_from: "plan".to_string(),
        routing_items: 12,
        nets: ["GND", "VCC", "SIG"]
            .iter()
            .map(|n| n.to_string())
            .collect(),
        rails: BTreeMap::new(),
        gate: None,
        gate_body: None,
        plan_revision: None,
        facts: facts(),
        route: None,
        at: None,
        attempt: 1,
        route_attempt: 1,
        records: Vec::new(),
        legs: Vec::new(),
        start_checkpoint: None,
        stop_reason: None,
        checkpoints: Vec::new(),
    }
}

fn keys(needs: &[Need]) -> Vec<&'static str> {
    needs.iter().map(|need| need.key).collect()
}

/// Walks the flow with every step finishing as done, as a leg would, and returns the
/// steps it ran, the ones it skipped, and where it stopped.
fn walk(flow: &mut FlowRecord) -> (Vec<Step>, Vec<Step>, Next) {
    let (mut ran, mut skipped) = (Vec::new(), Vec::new());
    for _ in 0..40 {
        match next(flow) {
            Next::Step(step) => {
                ran.push(step);
                let mut record = StepRecord::new(step, StepState::Done);
                record.after = Some(format!("after-{}", step.id()));
                flow.put(record);
            }
            Next::Skip(step, why) => {
                skipped.push(step);
                flow.put(StepRecord::skipped(step, why));
            }
            other => return (ran, skipped, other),
        }
    }
    panic!("the flow did not end");
}

// ─── Steps ───────────────────────────────────────────────────────────────────

#[test]
fn step_ids_follow_the_design_and_serialize_as_ids() {
    let ids: Vec<&str> = Step::ALL.iter().map(|step| step.id()).collect();
    assert_eq!(
        ids,
        [
            "P0", "P1", "P2", "P3", "P4", "L1", "L2", "L3", "L4", "R1", "R2", "R3", "F1", "F2a",
            "F2b", "F3", "F4"
        ]
    );
    for step in Step::ALL {
        assert_eq!(serde_json::to_value(step).unwrap(), json!(step.id()));
        assert!(!step.title().is_empty());
    }
    let writing: Vec<&str> = Step::ALL
        .iter()
        .filter(|s| s.writes())
        .map(|s| s.id())
        .collect();
    assert_eq!(writing, ["L2", "L3", "L4", "F1", "F2a", "F2b", "F3", "F4"]);
}

// ─── Approvals ───────────────────────────────────────────────────────────────

#[test]
fn approvals_read_strings_and_refuse_unknown_keys() {
    let approvals: Approvals = serde_json::from_value(json!({
        "placement": "apply", "delete_routing": "delete_including_locked", "pours": "skip",
        "pour_roles": "as_named", "routing": "apply_best"
    }))
    .unwrap();
    assert_eq!(
        approvals.pour_roles,
        Some(RolesAnswer::AsNamed(AsNamed::AsNamed))
    );
    let map: Approvals =
        serde_json::from_value(json!({ "pour_roles": { "GND": "ground", "X": "not_a_supply" } }))
            .unwrap();
    assert!(matches!(map.pour_roles, Some(RolesAnswer::Map(ref m)) if m.len() == 2));
    assert!(serde_json::from_value::<Approvals>(json!({ "placment": "apply" })).is_err());
    assert!(serde_json::from_value::<Approvals>(json!({ "routing": "yes" })).is_err());
    assert!(serde_json::from_value::<Approvals>(json!({ "pour_roles": "all" })).is_err());
}

#[test]
fn steps_default_to_running_and_take_only_their_strings() {
    let steps: Steps = serde_json::from_value(json!({ "routing": "skip" })).unwrap();
    assert_eq!(steps.placement, RunOrSkip::Run);
    assert_eq!(steps.pours, PlanOrSkip::Plan);
    assert_eq!(steps.routing, RunOrSkip::Skip);
    assert!(serde_json::from_value::<Steps>(json!({ "pours": "run" })).is_err());
    assert!(serde_json::from_value::<Steps>(json!({ "placement": true })).is_err());
    assert!(serde_json::from_value::<Steps>(json!({ "zones": "skip" })).is_err());
}

#[test]
fn merged_takes_new_keys_and_refuses_a_second_value() {
    let old = Approvals {
        placement: Some(PlacementAnswer::Apply),
        ..Approvals::default()
    };
    let answer = Approvals {
        placement: Some(PlacementAnswer::Apply),
        pours: Some(PoursAnswer::Add),
        ..Approvals::default()
    };
    let merged = old.merged(&answer, Gate::Plan).unwrap();
    assert_eq!(merged.placement, Some(PlacementAnswer::Apply));
    assert_eq!(merged.pours, Some(PoursAnswer::Add));
    let other = Approvals {
        placement: Some(PlacementAnswer::KeepCurrent),
        ..Approvals::default()
    };
    let refused = old.merged(&other, Gate::Plan).unwrap_err();
    assert!(refused.contains("`placement`"), "{refused}");
}

#[test]
fn pour_roles_may_be_answered_again_only_at_gate_a() {
    let old = Approvals {
        pour_roles: Some(RolesAnswer::AsNamed(AsNamed::AsNamed)),
        ..Approvals::default()
    };
    let new = Approvals {
        pour_roles: Some(RolesAnswer::Map(BTreeMap::from([(
            "VCC".to_string(),
            GivenRole::NotASupply,
        )]))),
        ..Approvals::default()
    };
    assert!(old.roles_changed(&new));
    assert!(!old.roles_changed(&old));
    assert_eq!(
        old.merged(&new, Gate::Plan).unwrap().pour_roles,
        new.pour_roles
    );
    assert!(old.merged(&new, Gate::Roles).is_err());
    assert!(old.merged(&new, Gate::Routing).is_err());
}

#[test]
fn routing_may_be_answered_again_only_at_gate_b() {
    let old = Approvals {
        routing: Some(RoutingAnswer::ApplyIfComplete),
        ..Approvals::default()
    };
    let new = Approvals {
        routing: Some(RoutingAnswer::ApplyBest),
        ..Approvals::default()
    };
    assert_eq!(
        old.merged(&new, Gate::Routing).unwrap().routing,
        Some(RoutingAnswer::ApplyBest)
    );
    assert!(old.merged(&new, Gate::Plan).is_err());
}

#[test]
fn routing_runs_needs_the_step_and_no_skip() {
    let steps = Steps::default();
    assert!(Approvals::default().routing_runs(&steps));
    assert!(all_approved().routing_runs(&steps));
    let skip = Approvals {
        routing: Some(RoutingAnswer::Skip),
        ..Approvals::default()
    };
    assert!(!skip.routing_runs(&steps));
    let no_step = Steps {
        routing: RunOrSkip::Skip,
        ..Steps::default()
    };
    assert!(!all_approved().routing_runs(&no_step));
}

// ─── Gate A ──────────────────────────────────────────────────────────────────

#[test]
fn gate_a_asks_every_key_with_its_recommendation() {
    let needs = needs_at_plan(&facts(), &Steps::default(), &Approvals::default());
    assert_eq!(
        keys(&needs),
        ["placement", "delete_routing", "pours", "routing"]
    );
    let recommended: Vec<Option<Value>> = needs.iter().map(|n| n.recommended.clone()).collect();
    assert_eq!(
        recommended,
        [
            Some(json!("apply")),
            Some(json!("delete")),
            Some(json!("add")),
            Some(json!("apply_if_complete"))
        ]
    );
}

#[test]
fn gate_a_follows_the_change() {
    let mut small = facts();
    small.change = Some("small".to_string());
    let needs = needs_at_plan(&small, &Steps::default(), &Approvals::default());
    assert_eq!(needs[0].key, "placement");
    assert_eq!(needs[0].recommended, Some(json!("keep_current")));
    for none in [Some("none".to_string()), None] {
        let mut same = facts();
        same.change = none;
        let needs = needs_at_plan(&same, &Steps::default(), &Approvals::default());
        assert!(!keys(&needs).contains(&"placement"));
    }
}

#[test]
fn gate_a_asks_no_deletion_without_routing_or_without_a_use_for_it() {
    let mut clean = facts();
    clean.unlocked_routing = 0;
    assert!(!keys(&needs_at_plan(
        &clean,
        &Steps::default(),
        &Approvals::default()
    ))
    .contains(&"delete_routing"));
    // Routing on the board, but nothing that needs it gone: the current placement is
    // kept and routing is skipped.
    let approved = Approvals {
        placement: Some(PlacementAnswer::KeepCurrent),
        routing: Some(RoutingAnswer::Skip),
        ..Approvals::default()
    };
    assert!(
        !keys(&needs_at_plan(&facts(), &Steps::default(), &approved)).contains(&"delete_routing")
    );
    // The same with only the placement kept: routing still runs, so it is asked.
    let placed = Approvals {
        placement: Some(PlacementAnswer::KeepCurrent),
        ..Approvals::default()
    };
    assert!(keys(&needs_at_plan(&facts(), &Steps::default(), &placed)).contains(&"delete_routing"));
}

#[test]
fn gate_a_recommends_deleting_locked_routing_when_it_blocks_or_moves() {
    for (blocks, moved) in [(true, false), (false, true)] {
        let mut locked = facts();
        locked.locked_routing = 3;
        locked.locked_blocks_export = blocks;
        locked.locked_on_moved = moved;
        let needs = needs_at_plan(&locked, &Steps::default(), &Approvals::default());
        let delete = needs.iter().find(|n| n.key == "delete_routing").unwrap();
        assert_eq!(delete.recommended, Some(json!("delete_including_locked")));
    }
}

#[test]
fn gate_a_recommends_no_pours_when_the_fill_check_is_not_clean() {
    let mut unclean = facts();
    unclean.fill_clean = false;
    let needs = needs_at_plan(&unclean, &Steps::default(), &Approvals::default());
    let pours = needs.iter().find(|n| n.key == "pours").unwrap();
    assert_eq!(pours.recommended, None);
    let mut none = facts();
    none.zones = 0;
    assert!(!keys(&needs_at_plan(
        &none,
        &Steps::default(),
        &Approvals::default()
    ))
    .contains(&"pours"));
}

#[test]
fn gate_a_does_not_ask_what_is_approved_in_advance() {
    assert!(needs_at_plan(&facts(), &Steps::default(), &all_approved()).is_empty());
}

#[test]
fn gate_a_asks_roles_left_from_names() {
    let mut named = facts();
    named.named_only = vec!["VCC".to_string()];
    let needs = needs_at_plan(&named, &Steps::default(), &Approvals::default());
    assert_eq!(needs[0].key, "pour_roles");
    assert_eq!(needs[0].recommended, Some(json!("as_named")));
}

#[test]
fn gate_r_asks_roles_from_names_only() {
    assert!(needs_roles(&[], &Approvals::default()).is_none());
    let need = needs_roles(&["VCC".to_string()], &Approvals::default()).unwrap();
    assert_eq!(need.key, "pour_roles");
    assert!(need.why.contains("VCC"));
    let given = Approvals {
        pour_roles: Some(RolesAnswer::AsNamed(AsNamed::AsNamed)),
        ..Approvals::default()
    };
    assert!(needs_roles(&["VCC".to_string()], &given).is_none());
}

// ─── Gate B ──────────────────────────────────────────────────────────────────

#[test]
fn gate_b_stops_only_where_the_approval_asks() {
    use RoutingAnswer::*;
    for (approval, pass, stops) in [
        (Ask, Some(true), true),
        (Ask, Some(false), true),
        (Ask, None, true),
        (ApplyIfComplete, Some(true), false),
        (ApplyIfComplete, Some(false), true),
        (ApplyIfComplete, None, true),
        (ApplyBest, Some(false), false),
        (ApplyBest, None, false),
        (Skip, Some(false), false),
    ] {
        assert_eq!(
            stops_for_routing(approval, pass),
            stops,
            "{approval:?} {pass:?}"
        );
    }
}

#[test]
fn gate_b_recommends_applying_only_when_every_gate_passed() {
    let ask = Approvals {
        routing: Some(RoutingAnswer::Ask),
        ..Approvals::default()
    };
    assert_eq!(
        needs_routing(&ask, Some(true)).unwrap().recommended,
        Some(json!("apply_best"))
    );
    let complete = Approvals {
        routing: Some(RoutingAnswer::ApplyIfComplete),
        ..Approvals::default()
    };
    assert_eq!(
        needs_routing(&complete, Some(false)).unwrap().recommended,
        None
    );
    assert!(needs_routing(&complete, Some(true)).is_none());
}

// ─── Refused combinations ────────────────────────────────────────────────────

fn refused(approved: &Approvals, facts: &PlanFacts) -> Option<String> {
    let nets: BTreeSet<String> = ["GND", "VCC"].iter().map(|n| n.to_string()).collect();
    let rails = BTreeMap::from([("GND".to_string(), "ground")]);
    refused_combination(approved, facts, &Steps::default(), &rails, &nets)
}

#[test]
fn the_approved_plan_is_not_refused() {
    assert_eq!(refused(&all_approved(), &facts()), None);
}

#[test]
fn placement_with_routing_kept_is_refused() {
    let approved = Approvals {
        delete_routing: Some(DeleteAnswer::Keep),
        routing: Some(RoutingAnswer::Skip),
        ..all_approved()
    };
    let reason = refused(&approved, &facts()).unwrap();
    assert!(
        reason.starts_with("placement: apply with delete_routing: keep"),
        "{reason}"
    );
    // Kept placement: no refusal, since routing is skipped.
    let kept = Approvals {
        placement: Some(PlacementAnswer::KeepCurrent),
        ..approved
    };
    assert_eq!(refused(&kept, &facts()), None);
}

#[test]
fn routing_with_routing_kept_is_refused() {
    let approved = Approvals {
        placement: Some(PlacementAnswer::KeepCurrent),
        delete_routing: Some(DeleteAnswer::Keep),
        ..all_approved()
    };
    let reason = refused(&approved, &facts()).unwrap();
    assert!(
        reason.starts_with("routing with delete_routing: keep"),
        "{reason}"
    );
}

#[test]
fn routing_with_a_refused_export_check_is_refused() {
    let mut blocked = facts();
    blocked.export_refused = Some("a pad shape the export cannot write".to_string());
    let reason = refused(&all_approved(), &blocked).unwrap();
    assert!(
        reason.contains("a pad shape the export cannot write"),
        "{reason}"
    );
    let skip = Approvals {
        routing: Some(RoutingAnswer::Skip),
        ..all_approved()
    };
    assert_eq!(refused(&skip, &blocked), None);
}

#[test]
fn routing_with_locked_routing_the_export_refuses_is_refused() {
    let mut locked = facts();
    locked.locked_routing = 2;
    locked.locked_blocks_export = true;
    let reason = refused(&all_approved(), &locked).unwrap();
    assert!(reason.contains("delete_including_locked"), "{reason}");
    let including = Approvals {
        delete_routing: Some(DeleteAnswer::DeleteIncludingLocked),
        ..all_approved()
    };
    assert_eq!(refused(&including, &locked), None);
}

#[test]
fn a_role_against_a_rail_or_for_a_missing_net_is_refused() {
    let against = Approvals {
        pour_roles: Some(RolesAnswer::Map(BTreeMap::from([(
            "GND".to_string(),
            GivenRole::Power,
        )]))),
        ..all_approved()
    };
    let reason = refused(&against, &facts()).unwrap();
    assert!(reason.contains("set_design_intent"), "{reason}");
    let same = Approvals {
        pour_roles: Some(RolesAnswer::Map(BTreeMap::from([(
            "GND".to_string(),
            GivenRole::Ground,
        )]))),
        ..all_approved()
    };
    assert_eq!(refused(&same, &facts()), None);
    let missing = Approvals {
        pour_roles: Some(RolesAnswer::Map(BTreeMap::from([(
            "NOPE".to_string(),
            GivenRole::Ground,
        )]))),
        ..all_approved()
    };
    assert!(refused(&missing, &facts()).unwrap().contains("NOPE"));
}

// ─── The revision ────────────────────────────────────────────────────────────

#[test]
fn the_plan_revision_binds_every_part_and_ignores_key_order() {
    let body = json!({ "a": 1, "b": { "c": [1, 2] } });
    let reordered: Value = serde_json::from_str(r#"{"b":{"c":[1,2]},"a":1}"#).unwrap();
    let base = plan_revision("f", Gate::Plan, "p", &body);
    assert_eq!(base, plan_revision("f", Gate::Plan, "p", &reordered));
    assert_eq!(base.len(), 64);
    for other in [
        plan_revision("g", Gate::Plan, "p", &body),
        plan_revision("f", Gate::Roles, "p", &body),
        plan_revision("f", Gate::Plan, "q", &body),
        plan_revision(
            "f",
            Gate::Plan,
            "p",
            &json!({ "a": 2, "b": { "c": [1, 2] } }),
        ),
    ] {
        assert_ne!(base, other);
    }
}

// ─── The live-step protocol ──────────────────────────────────────────────────

#[test]
fn a_live_step_runs_only_where_the_step_before_left_the_board() {
    assert_eq!(plan_live_step("a", "a"), LiveStart::Run);
    assert_eq!(
        plan_live_step("b", "a"),
        LiveStart::Changed {
            expected: "a".to_string(),
            found: "b".to_string()
        }
    );
}

#[test]
fn the_expected_board_is_the_last_written_steps_after() {
    let mut flow = flow();
    assert_eq!(flow.expected_before(Step::DeleteRouting), "plan");
    let mut deleted = StepRecord::new(Step::DeleteRouting, StepState::Done);
    deleted.after = Some("after-L2".to_string());
    flow.put(deleted);
    flow.put(StepRecord::skipped(Step::ApplyPlacement, "keep"));
    assert_eq!(flow.expected_before(Step::AddZones), "after-L2");
    assert_eq!(flow.expected_before(Step::DeleteRouting), "plan");
}

fn stopped(state: StepState) -> StepRecord {
    let mut record = StepRecord::new(Step::ApplyPlacement, state);
    record.before = Some("before".to_string());
    record.checkpoint = Some("cp-3".to_string());
    record
}

#[test]
fn resume_records_a_step_that_landed() {
    for state in [StepState::Calling, StepState::Failed, StepState::Uncertain] {
        assert_eq!(
            may_resume(&stopped(state), true, "elsewhere"),
            Ok(Resume::RecordDone)
        );
    }
}

#[test]
fn resume_runs_again_only_at_the_steps_own_start() {
    assert_eq!(
        may_resume(&stopped(StepState::Failed), false, "before"),
        Ok(Resume::RunAgain)
    );
    let refused = may_resume(&stopped(StepState::Uncertain), false, "elsewhere").unwrap_err();
    assert!(refused.starts_with("board_changed"), "{refused}");
    assert!(refused.contains("cp-3"), "{refused}");
    let mut unknown = stopped(StepState::Calling);
    unknown.before = None;
    assert!(may_resume(&unknown, false, "before").is_err());
}

#[test]
fn resume_only_saves_again_an_unsaved_step() {
    assert_eq!(
        may_resume(&stopped(StepState::Unsaved), false, "elsewhere"),
        Ok(Resume::SaveAgain)
    );
    assert_eq!(
        may_resume(&stopped(StepState::Unsaved), true, "before"),
        Ok(Resume::SaveAgain)
    );
}

// ─── The order ───────────────────────────────────────────────────────────────

#[test]
fn everything_approved_runs_every_step_in_order() {
    let mut flow = flow();
    flow.approvals = all_approved();
    flow.put(StepRecord::new(Step::Preflight, StepState::Done));
    // The route's facts arrive with R2; here they are known from the start.
    flow.route = Some(RouteFacts {
        all_gates_pass: Some(true),
        plane_stitching: true,
        reference_vias: true,
        length_rules: true,
    });
    let (ran, skipped, end) = walk(&mut flow);
    assert_eq!(end, Next::End);
    let expected: Vec<Step> = Step::ALL[1..]
        .iter()
        .copied()
        .filter(|step| SEARCH_WITH_POURS || *step != Step::PoursBefore)
        .collect();
    assert_eq!(ran, expected);
    assert_eq!(skipped.is_empty(), SEARCH_WITH_POURS, "{skipped:?}");
}

#[test]
fn gate_r_comes_right_after_the_preflight() {
    let mut flow = flow();
    flow.facts.named_only = vec!["VCC".to_string()];
    flow.put(StepRecord::new(Step::Preflight, StepState::Done));
    assert_eq!(next(&flow), Next::Gate(Gate::Roles));
    flow.approvals.pour_roles = Some(RolesAnswer::AsNamed(AsNamed::AsNamed));
    assert_eq!(next(&flow), Next::Step(Step::CountRouting));
}

#[test]
fn gate_a_comes_after_the_plan_and_before_any_write() {
    let mut flow = flow();
    flow.put(StepRecord::new(Step::Preflight, StepState::Done));
    let (ran, _, stop) = walk(&mut flow);
    assert_eq!(stop, Next::Gate(Gate::Plan));
    assert!(ran.iter().all(|step| !step.writes()), "{ran:?}");
    assert_eq!(ran.last(), Some(&Step::PoursAfter));
}

#[test]
fn gate_b_stops_before_the_apply_when_a_judge_gate_failed() {
    let mut flow = flow();
    flow.approvals = all_approved();
    flow.route = Some(RouteFacts {
        all_gates_pass: Some(false),
        ..RouteFacts::default()
    });
    flow.put(StepRecord::new(Step::Preflight, StepState::Done));
    let (ran, _, stop) = walk(&mut flow);
    assert_eq!(stop, Next::Gate(Gate::Routing));
    assert_eq!(ran.last(), Some(&Step::ImportCheck));
    flow.approvals.routing = Some(RoutingAnswer::Skip);
    let (ran, skipped, end) = walk(&mut flow);
    assert_eq!(end, Next::End);
    assert!(!ran.contains(&Step::ApplySession));
    assert!(skipped.contains(&Step::ApplySession));
    assert_eq!(
        ran,
        [Step::Finish],
        "the earlier writes still get their DRC and save"
    );
}

#[test]
fn a_route_that_needed_stitching_does_not_stop_when_every_gate_passed() {
    let mut flow = flow();
    flow.approvals = all_approved();
    flow.route = Some(RouteFacts {
        all_gates_pass: Some(true),
        plane_stitching: true,
        ..RouteFacts::default()
    });
    flow.put(StepRecord::new(Step::Preflight, StepState::Done));
    let (ran, skipped, end) = walk(&mut flow);
    assert_eq!(end, Next::End);
    assert!(ran.contains(&Step::ApplySession) && ran.contains(&Step::PlaneStitch));
    assert!(skipped.contains(&Step::ReferenceVias) && skipped.contains(&Step::LengthRules));
}

#[test]
fn every_skip_of_the_table_holds() {
    // Placement, pours and routing skipped: only the reads and nothing written.
    let mut flow = flow();
    flow.steps_asked = Steps {
        placement: RunOrSkip::Skip,
        pours: PlanOrSkip::Skip,
        routing: RunOrSkip::Skip,
    };
    flow.facts.change = None;
    flow.put(StepRecord::new(Step::Preflight, StepState::Done));
    let (ran, skipped, end) = walk(&mut flow);
    assert_eq!(end, Next::End);
    assert_eq!(ran, [Step::CountRouting, Step::Unchanged]);
    assert!(skipped.contains(&Step::Finish));
    for record in &flow.records {
        if record.state == StepState::Skipped {
            assert!(record.why.is_some(), "{:?}", record.step);
        }
    }
    // No routing on the board: P1 and L2 are skipped.
    let mut bare = self::flow();
    bare.routing_items = 0;
    bare.facts.unlocked_routing = 0;
    bare.approvals = all_approved();
    bare.put(StepRecord::new(Step::Preflight, StepState::Done));
    let (ran, skipped, _) = walk(&mut bare);
    assert!(skipped.contains(&Step::CountRouting) && skipped.contains(&Step::DeleteRouting));
    assert!(ran.contains(&Step::ApplyPlacement));
}

#[test]
fn a_small_change_kept_is_not_applied() {
    let mut flow = flow();
    flow.facts.change = Some("small".to_string());
    flow.approvals = Approvals {
        placement: Some(PlacementAnswer::KeepCurrent),
        ..all_approved()
    };
    flow.put(StepRecord::new(Step::Preflight, StepState::Done));
    let (ran, skipped, _) = walk(&mut flow);
    assert!(skipped.contains(&Step::ApplyPlacement));
    assert!(
        ran.contains(&Step::DeleteRouting),
        "routing runs, so the old routing goes"
    );
}

// ─── Summaries ───────────────────────────────────────────────────────────────

#[test]
fn the_summary_caps_its_lines_and_explains_every_skip() {
    let mut flow = flow();
    flow.approvals = all_approved();
    flow.route = Some(RouteFacts {
        all_gates_pass: Some(true),
        plane_stitching: true,
        reference_vias: true,
        length_rules: true,
    });
    flow.put(StepRecord::new(Step::Preflight, StepState::Done));
    walk(&mut flow);
    let mut route = StepRecord::new(Step::Route, StepState::Done);
    route.not_checked = vec!["lengths".to_string()];
    flow.put(route);
    flow.start_checkpoint = Some("cp-1".to_string());
    flow.checkpoints = vec!["cp-1".to_string(), "cp-2".to_string()];
    let route_summary = json!({ "pours": { "line": "Pours: as exported, and won." }, "lengths": { "line": "Lengths: no group." } });
    let body = summary(&flow, Some(&route_summary));
    let lines = body["lines"].as_array().unwrap();
    assert!(lines.len() <= SUMMARY_LINES);
    assert!(lines.contains(&json!("Pours: as exported, and won.")));
    assert_eq!(
        body["not_checked"],
        json!([{ "step": "R2", "what": "lengths" }])
    );
    assert_eq!(body["checkpoints"], 2);
    assert_eq!(
        body["way_back"]["preview"]["arguments"]["checkpoint_id"],
        "cp-1"
    );
    assert_eq!(body["way_back"]["preview"]["arguments"]["dry_run"], true);

    let mut skips = self::flow();
    skips.steps_asked.routing = RunOrSkip::Skip;
    skips.approvals = all_approved();
    skips.put(StepRecord::new(Step::Preflight, StepState::Done));
    walk(&mut skips);
    let body = summary(&skips, None);
    for row in body["skipped"].as_array().unwrap() {
        assert!(row["why"].is_string(), "{row}");
    }
}

#[test]
fn no_checkpoint_gives_no_way_back() {
    assert_eq!(way_back(&flow(), None), Value::Null);
}

#[test]
fn counts_come_from_the_steps_own_results() {
    assert_eq!(
        counts_of(
            Step::DeleteRouting,
            &json!({ "removed": { "total": 3, "tracks": 2, "arcs": 0, "vias": 1 } })
        ),
        json!({ "total": 3, "tracks": 2, "arcs": 0, "vias": 1 })
    );
    assert_eq!(
        counts_of(
            Step::LengthRules,
            &json!({ "rules": [{ "action": "created" }, { "action": "replaced" }, { "action": "created" }] })
        ),
        json!({ "rules": { "created": 2, "replaced": 1 } })
    );
    assert_eq!(
        counts_of(Step::Export, &json!({ "anything": 1 })),
        json!({})
    );
}

#[test]
fn unconnected_is_read_from_the_drc_tally() {
    let drc = json!({ "by_rule": [
        { "rule": "unconnected_items", "severity": "error", "count": 4 },
        { "rule": "clearance", "severity": "error", "count": 1 }
    ]});
    assert_eq!(unconnected_of(&drc), Some(4));
    assert_eq!(unconnected_of(&json!({ "by_rule": [] })), Some(0));
    assert_eq!(unconnected_of(&json!({})), None);
}
