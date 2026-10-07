#![forbid(unsafe_code)]

use mir_check::{ProofStatus, Report, cli};
use serde_json::json;

fn report(status: ProofStatus, detail: &str) -> Report {
    let status = serde_json::to_value(status).unwrap();
    let mut report: Report = serde_json::from_value(json!({
        "schema_version": 9,
        "compiler": "fixture",
        "crate_name": "packet",
        "target": "host",
        "panic_strategy": "abort",
        "overflow_checks": true,
        "mir_phase": "runtime",
        "rustc_arguments": [],
        "functions": [{
            "name": "decode",
            "source": {"file": "packet.rs", "line": 4, "column": 1},
            "basic_blocks": 2,
            "arguments": 1,
            "contracts": [],
            "sites": [],
            "local_calls": [],
            "proof": {
                "status": status,
                "assumptions": [],
                "inputs": {"index": "v0"},
                "models": [],
                "analyzed_bodies": ["decode", "read", "index"],
                "obligations": [{
                    "function": "index",
                    "source": {"file": "buffer.rs", "line": 18, "column": 7},
                    "kind": "panic_safety",
                    "status": status,
                    "detail": detail,
                    "query": "(check-sat)",
                    "model": null,
                    "call_chain": ["decode", "read", "index"]
                }]
            }
        }],
        "traces": [],
        "coverage": {
            "inventoried_bodies": 0,
            "selected_roots": 0,
            "proved": 0,
            "refuted": 0,
            "unknown": 0,
            "unselected_bodies": 0,
            "interpreted_instances": 0,
            "gaps": []
        }
    }))
    .unwrap();
    report.coverage = mir_check::coverage(&report);
    report
}

#[test]
fn failures_name_the_condition_location_and_symbolic_call_chain() {
    let report = report(ProofStatus::Refuted, "index must be below length");
    for verbose in [false, true] {
        let display = cli::render_report(&report, verbose, false);
        assert!(display.contains("panic condition: index must be below length"));
        assert!(display.contains("at buffer.rs:18:7 in index"));
        assert!(display.contains("call chain: decode -> read -> index"));
        assert!(display.contains("symbolic counterexample; runtime panic unconfirmed"));
        assert!(!display.contains("\x1b["));
        assert!(!cli::accepted(&report, false));
    }
    assert!(cli::render_report(&report, false, true).contains("\x1b[31mREFUTED"));
}

#[test]
fn saved_reports_without_a_call_chain_do_not_invent_intermediate_calls() {
    let mut value = serde_json::to_value(report(ProofStatus::Refuted, "bounds check")).unwrap();
    value["schema_version"] = json!(8);
    value["functions"][0]["proof"]["obligations"][0]
        .as_object_mut()
        .unwrap()
        .remove("call_chain");
    let report: Report = serde_json::from_value(value).unwrap();
    let display = cli::render_report(&report, false, false);
    assert!(display.contains("root: decode; failing function: index (call chain not recorded)"));
    assert!(!display.contains("decode -> read -> index"));
    assert!(display.contains("runtime panic unconfirmed"));
}

#[test]
fn an_incomplete_analysis_points_to_the_budget_that_actually_blocked_it() {
    for (detail, flag) in [
        ("32-frame call-depth limit reached", "--max-call-depth"),
        ("symbolic execution step limit reached", "--max-steps"),
        ("symbolic query size limit reached", "--max-query-bytes"),
        (
            "symbolic root exceeded the 20-second execution budget",
            "--root-timeout-secs",
        ),
        ("Z3 returned timeout", "--solver-timeout-ms"),
    ] {
        let display = cli::render_report(&report(ProofStatus::Unknown, detail), false, false);
        assert!(display.contains(flag), "{display}");
        assert!(display.contains("at buffer.rs:18:7 in index"));
        assert!(!display.contains("runtime panic unconfirmed"));
    }
    let display = cli::render_report(
        &report(ProofStatus::Unknown, "raw-pointer dereference unsupported"),
        false,
        false,
    );
    assert!(display.contains("larger budgets cannot model unsupported code"));
    let display = cli::render_report(
        &report(
            ProofStatus::Unknown,
            "MIR body unavailable for packet::timeout",
        ),
        false,
        false,
    );
    assert!(display.contains("Rebuild with dependency MIR retention"));
    assert!(!display.contains("--solver-timeout-ms"));
}

#[test]
fn a_refuted_root_still_explains_an_unknown_obligation_that_follows_it() {
    let mut report = report(ProofStatus::Refuted, "bounds check");
    let mut blocker =
        serde_json::to_value(&report.functions[0].proof.as_ref().unwrap().obligations[0]).unwrap();
    blocker["status"] = json!("unknown");
    blocker["kind"] = json!("unsupported");
    blocker["detail"] = json!("raw-pointer dereference unsupported");
    report.functions[0]
        .proof
        .as_mut()
        .unwrap()
        .obligations
        .push(serde_json::from_value(blocker).unwrap());
    report.coverage = mir_check::coverage(&report);
    let display = cli::render_report(&report, false, false);
    assert!(display.contains("analysis also incomplete:"));
    assert!(display.contains("blocked by: raw-pointer dereference unsupported"));
    assert!(display.contains("runtime panic unconfirmed"));
    assert_eq!(report.coverage.unknown, 0);
    assert_eq!(report.coverage.gaps.len(), 1);
}

#[test]
fn proven_roots_do_not_show_a_failure_explanation() {
    let report = report(ProofStatus::Proved, "bounds check");
    assert!(cli::accepted(&report, false));
    for verbose in [false, true] {
        let display = cli::render_report(&report, verbose, false);
        assert!(!display.contains("panic condition:"));
        assert!(!display.contains("runtime panic unconfirmed"));
        assert!(!display.contains("next:"));
    }
}

#[test]
fn native_evidence_distinguishes_matching_panics_and_unconfirmed_inputs() {
    use mir_check::replay::{ReplayResult, ReplaySource, ReplayStatus};

    for (status, label) in [
        (ReplayStatus::ConfirmedPanic, "native replay panicked"),
        (ReplayStatus::NotReproduced, "native replay did not panic"),
        (ReplayStatus::Unsupported, "native replay unsupported"),
        (
            ReplayStatus::ToolFailure,
            "native replay could not complete",
        ),
    ] {
        for matches_obligation in [false, true] {
            let mut report = report(ProofStatus::Refuted, "bounds check");
            let obligation = &mut report.functions[0].proof.as_mut().unwrap().obligations[0];
            obligation.replay = Some(ReplayResult {
                status,
                detail: "Replay evidence is separate from proof status".to_owned(),
                inputs: [("index".to_owned(), "8u8".to_owned())].into(),
                panic_source: Some(ReplaySource {
                    file: "buffer.rs".to_owned(),
                    line: 18,
                    column: 7,
                }),
                matches_obligation,
                panic_strategy: "unwind (native replay only)".to_owned(),
                panic_message: Some("index out of bounds".to_owned()),
                uncontrolled_abstractions: Vec::new(),
            });
            for verbose in [false, true] {
                let display = cli::render_report(&report, verbose, false);
                assert!(display.contains(label), "{display}");
                assert!(display.contains("replay inputs: index = 8u8"));
                assert!(display.contains("replay panic strategy: unwind (native replay only)"));
                assert!(!display.contains("query abstraction choices were not forced"));
                if status == ReplayStatus::ConfirmedPanic {
                    assert!(display.contains("native panic at buffer.rs:18:7"));
                    assert!(display.contains("native panic message: index out of bounds"));
                    if matches_obligation {
                        assert!(display.contains("panic site matches the obligation"));
                    } else {
                        assert!(display.contains("panic site differs or could not be matched"));
                    }
                } else {
                    assert!(!display.contains("panic site matches the obligation"));
                }
            }
            assert!(!cli::accepted(&report, true));
            let totals = cli::render_totals(&[report], false, false, 1.0);
            assert!(totals.contains("Native replay:"));
            assert!(totals.contains("Replay evidence does not change proof status"));
            assert!(totals.contains("Run failed"));
        }
    }
}

#[test]
fn concrete_counterexample_inputs_can_be_displayed_without_running_the_function() {
    use mir_check::replay::{ReplayArgument, ReplayArguments, ReplayValue};

    let mut report = report(ProofStatus::Refuted, "bounds check");
    let proof = report.functions[0].proof.as_mut().unwrap();
    proof.replay_inputs = Some(ReplayArguments {
        source_files: Default::default(),
        working_directory: None,
        build_environment: Default::default(),
        arguments: vec![ReplayArgument {
            name: "index".to_owned(),
            rust_type: "u8".to_owned(),
            value: ReplayValue::Integer {
                symbol: "v0".to_owned(),
                bits: 8,
                signed: false,
            },
        }],
        unsupported: None,
    });
    proof.obligations[0].model = Some("((define-fun v0 () (_ BitVec 8) #x08))".to_owned());
    let display = cli::render_report(&report, false, false);
    assert!(
        display.contains("counterexample inputs: index ="),
        "{display}"
    );
    assert!(display.contains("8u8"), "{display}");
    assert!(display.contains("runtime panic unconfirmed"));
    assert!(!display.contains("native replay panicked"));
    report.functions[0].proof.as_mut().unwrap().obligations[0].model = Some("()".to_owned());
    let display = cli::render_report(&report, false, false);
    assert!(!display.contains("counterexample inputs:"));
}

#[test]
fn a_refuted_contract_is_not_presented_as_a_confirmed_panic() {
    let mut report = report(ProofStatus::Refuted, "result must be below 8");
    report.functions[0].proof.as_mut().unwrap().obligations[0].kind =
        mir_check::ObligationKind::Postcondition;
    let display = cli::render_report(&report, false, false);
    assert!(display.contains("postcondition: result must be below 8"));
    assert!(display.contains("runtime failure unconfirmed"));
    assert!(!display.contains("runtime panic unconfirmed"));
}

#[test]
fn report_reading_preserves_machine_output_and_accepts_legacy_schemas() {
    let directory =
        std::env::temp_dir().join(format!("mir-check-report-ux-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("report.json");
    for schema in [7, 8, 9] {
        let mut value = serde_json::to_value(report(ProofStatus::Refuted, "bounds check")).unwrap();
        value["schema_version"] = json!(schema);
        if schema < 9 {
            value["functions"][0]["proof"]["obligations"][0]
                .as_object_mut()
                .unwrap()
                .remove("call_chain");
        }
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_mir-check"))
            .args(["report", "--quiet", "--jsonl", "-", "--color", "always"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let saved: Report = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(saved.schema_version, schema);
        assert_eq!(saved.coverage.refuted, 1);
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("runtime panic unconfirmed"));
        assert!(stderr.contains("at buffer.rs:18:7 in index"));
        assert!(stderr.contains("\x1b[31mREFUTED"));
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failure_reports_explain_query_abstractions_without_claiming_runtime_confirmation() {
    let mut report = report(ProofStatus::Refuted, "atomic claim failed");
    report.functions[0].proof.as_mut().unwrap().obligations[0].abstraction_reasons =
        vec!["shared atomic reads allow arbitrary old values".to_owned()];
    let display = cli::render_report(&report, false, false);
    assert!(
        display.contains("abstraction in query: shared atomic reads allow arbitrary old values")
    );
    assert!(display.contains("runtime panic unconfirmed"));
    assert!(display.contains("Validate these query choices against the root's execution"));
}

#[test]
fn native_replay_records_uncontrolled_choices_even_when_it_confirms_a_panic() {
    use mir_check::replay::{ReplayResult, ReplayStatus};

    let choices = vec![
        "shared atomic reads allow arbitrary old values".to_owned(),
        "weak compare-exchange permits spurious failure".to_owned(),
        "arithmetic NaN encodings are conservative".to_owned(),
    ];
    for status in [ReplayStatus::ConfirmedPanic, ReplayStatus::NotReproduced] {
        let mut report = report(ProofStatus::Refuted, "atomic claim failed");
        let obligation = &mut report.functions[0].proof.as_mut().unwrap().obligations[0];
        obligation.abstraction_reasons = choices.clone();
        obligation.replay = Some(ReplayResult {
            status,
            detail: "native execution used its own environment".to_owned(),
            inputs: Default::default(),
            panic_source: None,
            panic_message: None,
            matches_obligation: false,
            panic_strategy: "abort".to_owned(),
            uncontrolled_abstractions: choices.clone(),
        });
        for verbose in [false, true] {
            let display = cli::render_report(&report, verbose, false);
            assert!(display.contains("query abstraction choices were not forced"));
            assert_eq!(
                display.contains("A normal return does not validate every shared-state history"),
                status == ReplayStatus::NotReproduced
            );
            assert!(!display.contains("false positive proved"));
            assert!(!cli::accepted(&report, true));
        }
        let serialized = serde_json::to_value(&report).unwrap();
        let evidence = &serialized["functions"][0]["proof"]["obligations"][0]["replay"];
        assert_eq!(evidence["uncontrolled_abstractions"], json!(choices));
        let mut legacy = serialized;
        legacy["functions"][0]["proof"]["obligations"][0]["replay"]
            .as_object_mut()
            .unwrap()
            .remove("uncontrolled_abstractions");
        let legacy: Report = serde_json::from_value(legacy).unwrap();
        assert!(
            legacy.functions[0].proof.as_ref().unwrap().obligations[0]
                .replay
                .as_ref()
                .unwrap()
                .uncontrolled_abstractions
                .is_empty()
        );
    }
}
