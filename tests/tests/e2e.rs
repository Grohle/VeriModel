//! End-to-end tests on real IFC files. See `tests/fixtures/README.md`.

use std::path::Path;

use verimodel_core::config::Config;
use verimodel_core::model::{RuleStatus, Severity, Verdict};
use verimodel_tests::{bcf_topics, example, fixture, read, run, skip_ifc};

fn finding_ids(o: &verimodel_core::run::RunOutcome, rule: &str) -> Vec<String> {
    o.report.findings.iter().filter(|f| f.rule_id == rule).map(|f| f.global_id.clone().unwrap_or_default()).collect()
}

// ---------------------------------------------------------------------------
// Golden fixtures
// ---------------------------------------------------------------------------

#[test]
fn broken_building_fails_with_the_expected_findings() {
    if skip_ifc() {
        return;
    }
    let model = read("broken_building.ifc", None);
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::from_yaml(
        "requirements:\n  required_psets:\n    - pset: Pset_WallCommon\n      applies_to: [IfcWall]\n      properties: [FireRating]\n",
    )
    .unwrap();
    let o = run(&model, &cfg, None, None, dir.path());
    let s = &o.report.summary;
    assert_eq!(s.verdict, Some(Verdict::Fail));
    assert!(s.score < 100);
    // 1. duplicate GlobalId: both walls
    assert_eq!(finding_ids(&o, "ID-001").len(), 2, "{:#?}", o.report.findings);
    // 2. invalid GlobalId on the slab
    assert_eq!(finding_ids(&o, "ID-002"), vec!["BADID01".to_string()]);
    // 3. orphan column
    let orphan = finding_ids(&o, "SP-001");
    assert_eq!(orphan.len(), 1);
    assert_eq!(o.report.findings.iter().find(|f| f.rule_id == "SP-001").unwrap().name.as_deref(), Some("Column-Orphan"));
    // 4. zero-volume wall
    let zero = o.report.findings.iter().filter(|f| f.rule_id == "QT-001").collect::<Vec<_>>();
    assert_eq!(zero.len(), 1);
    assert_eq!(zero[0].name.as_deref(), Some("Wall-Zero"));
    // 5. far from origin
    let far = o.report.findings.iter().filter(|f| f.rule_id == "SP-004").collect::<Vec<_>>();
    assert_eq!(far.len(), 1);
    assert_eq!(far[0].name.as_deref(), Some("Beam-Far"));
    // 6. missing FireRating from config-generated rule (2 walls with the pset but no property)
    assert_eq!(finding_ids(&o, "CFG-PSET-001-01").len(), 2);
    // 7. empty property value
    assert_eq!(finding_ids(&o, "PR-001").len(), 2);
    // 8. duplicate names
    assert_eq!(finding_ids(&o, "ID-003").len(), 2);
    // every finding carries a fix hint and an element reference
    for f in &o.report.findings {
        assert!(f.fix_hint.is_some(), "{f:?}");
        assert!(f.global_id.is_some(), "{f:?}");
    }
    // outputs
    for name in ["report.html", "report.json", "failures.csv", "qto.csv", "qto.xlsx", "issues.bcfzip", "issues.csv", "run.json", "audit.log", "project.yaml"] {
        assert!(dir.path().join(name).is_file(), "missing {name}");
    }
    // BCF: one topic per ERROR/WARNING finding
    let expected = o.report.findings.iter().filter(|f| f.severity != Severity::Info).count();
    assert_eq!(bcf_topics(&dir.path().join("issues.bcfzip")).len(), expected);
    assert_eq!(o.bcf.topics, expected);
}

#[test]
fn qto_of_broken_building_matches_hand_computed_values() {
    if skip_ifc() {
        return;
    }
    let model = read("broken_building.ifc", None);
    let cfg = Config::from_yaml("qto:\n  group_by: [type]\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let o = run(&model, &cfg, None, None, dir.path());
    let row = |t: &str| o.report.qto.rows.iter().find(|r| r.group[0] == t).unwrap_or_else(|| panic!("row {t}"));
    // walls: 5000x3000x200 + 4000x3000x200 + 3000x3000x0 (mm) = 3.0 + 2.4 + 0 m3, from geometry (no base quantities)
    let walls = row("IfcWall");
    assert_eq!(walls.count, 3);
    assert!((walls.volume_m3 - 5.4).abs() < 1e-6, "{}", walls.volume_m3);
    assert_eq!(walls.volume_sources.get("geometry"), Some(&3));
    // slab 6000x6000x250 mm = 9.0 m3
    assert!((row("IfcSlab").volume_m3 - 9.0).abs() < 1e-6);
    // column 300x300x3000 = 0.27 m3 ; door 900x100x2100 = 0.189 m3 ; beam 4000x200x400 = 0.32 m3
    assert!((row("IfcColumn").volume_m3 - 0.27).abs() < 1e-6);
    assert!((row("IfcDoor").volume_m3 - 0.189).abs() < 1e-6);
    assert!((row("IfcBeam").volume_m3 - 0.32).abs() < 1e-4);
    assert!((o.report.qto.total_volume_m3 - (5.4 + 9.0 + 0.27 + 0.189 + 0.32)).abs() < 1e-4);
    assert_eq!(o.report.qto.samples.len(), 5);
    assert!(!o.report.qto.method.is_empty());
}

#[test]
fn public_samples_pass_or_warn_but_never_crash() {
    if skip_ifc() {
        return;
    }
    for name in ["bsi_ifc2x3_building_architecture.ifc", "bsi_ifc4_building_structural.ifc", "bsi_ifc4_building_hvac.ifc", "bsi_ifc4x3_infra_bridge.ifc"] {
        let model = read(name, None);
        let dir = tempfile::tempdir().unwrap();
        let o = run(&model, &Config::default(), None, None, dir.path());
        assert!(o.report.summary.verdict.is_some(), "{name}");
        assert_eq!(o.report.model.element_count, model.elements.len());
        assert!(o.report.rules.iter().all(|r| r.status != RuleStatus::Skipped || r.skipped_reason.is_some()), "{name}: skipped rules need a reason");
        assert!(dir.path().join("report.html").metadata().unwrap().len() > 1000, "{name}");
        // identity rules pass on the official samples
        for id in ["ID-001", "ID-002"] {
            assert_eq!(o.report.rules.iter().find(|r| r.rule_id == id).unwrap().status, RuleStatus::Pass, "{name} {id}");
        }
    }
}

#[test]
fn architecture_sample_qto_reconciles_with_base_quantities() {
    if skip_ifc() {
        return;
    }
    // Hand check: the IFC2x3 sample declares Qto_WallBaseQuantities.NetVolume on its 4 walls.
    let model = read("bsi_ifc2x3_building_architecture.ifc", None);
    let expected: f64 = model.elements.iter().filter(|e| e.ifc_type == "IfcWall").map(|e| e.quantity("NetVolume").unwrap()).sum();
    assert!(expected > 0.0);
    let cfg = Config::from_yaml("qto:\n  group_by: [type]\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let o = run(&model, &cfg, None, None, dir.path());
    let walls = o.report.qto.rows.iter().find(|r| r.group[0] == "IfcWall").unwrap();
    assert_eq!(walls.count, 4);
    assert!((walls.volume_m3 - expected).abs() < 1e-9);
    assert_eq!(walls.volume_sources.get("base_quantity"), Some(&4));
}

// ---------------------------------------------------------------------------
// Genericity: same IFC, two configs, different correct results
// ---------------------------------------------------------------------------

#[test]
fn same_ifc_two_configs_give_different_results() {
    if skip_ifc() {
        return;
    }
    let model = read("bsi_ifc4_building_structural.ifc", None);
    let lenient = Config::default();
    let strict = Config::from_yaml(
        "requirements:\n  required_psets:\n    - pset: Pset_BeamCommon\n      applies_to: [IfcBeam]\n      properties: [LoadBearing]\n\
         tolerances:\n  far_from_origin_m: 1\n\
         classification:\n  required: true\n\
         naming:\n  patterns:\n    IfcBeam: '^B-[0-9]{3}$'\n",
    )
    .unwrap();
    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    let a = run(&model, &lenient, None, None, d1.path());
    let b = run(&model, &strict, None, None, d2.path());
    assert!(b.report.summary.errors > a.report.summary.errors, "strict config must add errors: {} vs {}", b.report.summary.errors, a.report.summary.errors);
    assert!(b.report.findings.iter().any(|f| f.rule_id == "CFG-PSET-001"));
    assert!(b.report.findings.iter().any(|f| f.rule_id == "CL-001"));
    assert!(b.report.findings.iter().any(|f| f.rule_id == "NAM-001"));
    assert!(b.report.findings.iter().any(|f| f.rule_id == "SP-004"));
    assert!(!a.report.findings.iter().any(|f| f.rule_id == "CL-001"));
    assert!(a.report.rules.iter().find(|r| r.rule_id == "NAM-001").unwrap().status == RuleStatus::Skipped);
    assert_ne!(a.report.config, b.report.config);
}

#[test]
fn example_configs_load_and_run() {
    if skip_ifc() {
        return;
    }
    let model = read("bsi_ifc4x3_infra_bridge.ifc", None);
    for (ex, ifc_ok) in [("residential", true), ("office", true), ("infrastructure", true)] {
        let path = example(&format!("{ex}/project.yaml"));
        let cfg = Config::from_file(&path).unwrap_or_else(|e| panic!("{ex}: {e}"));
        let dir = tempfile::tempdir().unwrap();
        let o = run(&model, &cfg, path.parent(), None, dir.path());
        assert!(o.report.summary.verdict.is_some() == ifc_ok, "{ex}");
    }
    // the office example rejects nothing by schema but the infrastructure one accepts only IFC4X3
    let infra = Config::from_file(&example("infrastructure/project.yaml")).unwrap();
    assert_eq!(infra.project.ifc_versions, vec!["IFC4X3".to_string()]);
    let arch = read("bsi_ifc2x3_building_architecture.ifc", None);
    let dir = tempfile::tempdir().unwrap();
    let o = run(&arch, &infra, example("infrastructure").as_path().into(), None, dir.path());
    assert!(o.report.findings.iter().any(|f| f.rule_id == "SCH-001"), "IFC2X3 file must fail the IFC4X3-only project");
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn two_runs_produce_identical_outputs_except_timestamps() {
    if skip_ifc() {
        return;
    }
    let model = read("broken_building.ifc", Some(&example("sample.ids")));
    let cfg = Config::from_file(&example("residential/project.yaml")).unwrap();
    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    run(&model, &cfg, example("residential").as_path().into(), Some(&example("sample.ids")), d1.path());
    run(&model, &cfg, example("residential").as_path().into(), Some(&example("sample.ids")), d2.path());
    for name in ["report.json", "failures.csv", "qto.csv", "qto.xlsx", "qto.json", "report.html", "issues.csv", "project.yaml"] {
        let a = std::fs::read(d1.path().join(name)).unwrap();
        let b = std::fs::read(d2.path().join(name)).unwrap();
        assert!(a == b, "{name} differs between two identical runs");
    }
    // the BCF differs only by creation dates: same topics, same GUIDs
    assert_eq!(bcf_topics(&d1.path().join("issues.bcfzip")), bcf_topics(&d2.path().join("issues.bcfzip")));
    // reading the model twice gives the same facts
    let model2 = read("broken_building.ifc", Some(&example("sample.ids")));
    assert_eq!(serde_json::to_string(&model).unwrap(), serde_json::to_string(&model2).unwrap());
}

// ---------------------------------------------------------------------------
// IDS
// ---------------------------------------------------------------------------

#[test]
fn ids_violations_are_reported_with_the_requirement_quoted() {
    if skip_ifc() {
        return;
    }
    let ids = example("sample.ids");
    let model = read("broken_building.ifc", Some(&ids));
    let dir = tempfile::tempdir().unwrap();
    let o = run(&model, &Config::default(), None, Some(&ids), dir.path());
    let ids_report = o.report.ids.as_ref().expect("ids section");
    assert!(ids_report.specifications_total >= 3);
    let ids_findings: Vec<_> = o.report.findings.iter().filter(|f| f.rule_id.starts_with("IDS-")).collect();
    assert!(!ids_findings.is_empty());
    // The walls lack FireRating: the quoted requirement must mention it
    let fr = ids_findings.iter().find(|f| f.expected.as_deref().map(|s| s.contains("FireRating")).unwrap_or(false));
    assert!(fr.is_some(), "{ids_findings:#?}");
    assert!(matches!(fr.unwrap().source, verimodel_core::model::RuleSource::Ids { .. }));
    assert!(o.report.rules.iter().any(|r| r.rule_id.starts_with("IDS-") && r.status == RuleStatus::Fail));
    // run.json records the IDS hash
    assert!(o.ledger.ids_sha256.as_ref().map(|s| s.len() == 64).unwrap_or(false));
}

// ---------------------------------------------------------------------------
// Schema coverage
// ---------------------------------------------------------------------------

#[test]
fn ifc2x3_ifc4_and_ifc4x3_all_work() {
    if skip_ifc() {
        return;
    }
    let cases = [("bsi_ifc2x3_building_architecture.ifc", "IFC2X3"), ("bsi_ifc4_building_structural.ifc", "IFC4"), ("bsi_ifc4x3_infra_bridge.ifc", "IFC4X3")];
    for (name, schema) in cases {
        let model = read(name, None);
        assert_eq!(model.info.schema, schema);
        let dir = tempfile::tempdir().unwrap();
        let o = run(&model, &Config::default(), None, None, dir.path());
        assert_eq!(o.ledger.schema, schema);
        assert!(o.report.qto.total_count > 0, "{name} has takeoff items");
    }
    let _ = Path::new(&fixture("broken_building.ifc"));
}

/// Not a test: writes a full run of the broken fixture (with the sample IDS and the
/// residential example) to `$VERIMODEL_DEMO_OUT` so humans can open the outputs.
/// `cargo test -p verimodel-tests -- --ignored write_demo_outputs`
#[test]
#[ignore]
fn write_demo_outputs() {
    let Ok(out) = std::env::var("VERIMODEL_DEMO_OUT") else {
        eprintln!("set VERIMODEL_DEMO_OUT");
        return;
    };
    let ids = example("sample.ids");
    let cfg_path = example("residential/project.yaml");
    let cfg = Config::from_file(&cfg_path).unwrap();
    for name in ["broken_building.ifc", "bsi_ifc2x3_building_architecture.ifc", "bsi_ifc4x3_infra_bridge.ifc"] {
        let model = read(name, Some(&ids));
        let dir = Path::new(&out).join(name.trim_end_matches(".ifc"));
        let o = run(&model, &cfg, cfg_path.parent(), Some(&ids), &dir);
        eprintln!("{name}: score {} verdict {:?} errors {} warnings {} -> {}", o.report.summary.score, o.report.summary.verdict, o.report.summary.errors, o.report.summary.warnings, dir.display());
    }
}

#[test]
fn demo_building_shows_a_realistic_mixed_review() {
    if skip_ifc() {
        return;
    }
    let cfg_path = example("demo/project.yaml");
    let cfg = Config::from_file(&cfg_path).unwrap();
    let ids = cfg.ids_path(cfg_path.parent()).unwrap();
    let path = example("demo/demo_building.ifc");
    let model = verimodel_ifc_reader::read_model(&path, &verimodel_ifc_reader::ReadOptions { ids_path: Some(ids.clone()), ..Default::default() }).unwrap();
    assert_eq!(model.info.storeys, vec!["Level 1", "Level 2", "Roof"]);
    let dir = tempfile::tempdir().unwrap();
    let o = run(&model, &cfg, cfg_path.parent(), Some(&ids), dir.path());
    let s = &o.report.summary;
    assert_eq!(s.verdict, Some(Verdict::Fail), "{:#?}", o.report.findings);
    assert!(s.score >= 85 && s.score < 100, "score {}", s.score);
    let by_rule = |id: &str| o.report.findings.iter().filter(|f| f.rule_id == id).map(|f| f.name.clone().unwrap_or_default()).collect::<Vec<_>>();
    assert_eq!(by_rule("SP-001"), vec!["C-205"]);
    assert_eq!(by_rule("CFG-PSET-002-01"), vec!["D-103"]);
    assert_eq!(by_rule("PR-001"), vec!["W-204"]);
    assert_eq!(by_rule("CL-001"), vec!["MEP-102"]);
    assert_eq!(by_rule("ID-003").len(), 2);
    assert!(o.report.findings.iter().all(|f| f.rule_id != "ID-001" && f.rule_id != "ID-002"));
    // quantities come from base quantities, not geometry
    let walls = o.report.qto.rows.iter().filter(|r| r.group[0] == "IfcWall").map(|r| r.count).sum::<usize>();
    assert_eq!(walls, 9);
    assert!(o.report.qto.rows.iter().filter(|r| r.group[0] == "IfcWall").all(|r| r.volume_sources.get("base_quantity") == Some(&r.count)));
    assert!(o.report.qto.total_cost.unwrap_or(0.0) > 0.0);
}
