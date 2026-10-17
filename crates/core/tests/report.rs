//! Integration tests for the report writers: every output file is produced,
//! user data is escaped, and every writer is byte-for-byte deterministic.

use std::collections::BTreeMap;
use std::path::Path;

use verimodel_core::config::{Config, Language};
use verimodel_core::i18n::Strings;
use verimodel_core::model::{
    ClashReport, CostLine, Finding, IdsFailedEntity, IdsReport, IdsRequirementResult,
    IdsSpecificationResult, ModelInfo, QtoReport, QtoRow, QtoSample, QuantitySource, RuleResult,
    RuleSource, RuleStatus, RunReport, Severity, Summary, ToolInfo, Verdict,
};
use verimodel_core::report;

fn finding(rule: &str, sev: Severity, gid: &str, name: &str, source: RuleSource) -> Finding {
    Finding {
        rule_id: rule.into(),
        severity: sev,
        global_id: Some(gid.into()),
        step_id: Some(42),
        ifc_type: Some("IfcWall".into()),
        name: Some(name.into()),
        storey: Some("Level 1".into()),
        message: format!("{rule} failed, with a comma"),
        expected: Some("a value".into()),
        actual: Some("\"nothing\"".into()),
        fix_hint: Some("Fill it in".into()),
        source,
    }
}

fn sources(base: usize, geom: usize) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    if base > 0 {
        m.insert("base_quantity".to_string(), base);
    }
    if geom > 0 {
        m.insert("geometry".to_string(), geom);
    }
    m
}

fn sample_report() -> RunReport {
    let pack = RuleSource::RulePack {
        pack: "base".into(),
    };
    let ids_src = RuleSource::Ids {
        specification: "Walls".into(),
        requirement: "FireRating".into(),
    };
    let rules = vec![
        RuleResult {
            rule_id: "GEO-001".into(),
            category: "geometry".into(),
            description: "Elements have geometry".into(),
            severity: Severity::Error,
            status: RuleStatus::Skipped,
            elements_checked: 0,
            failures: 0,
            skipped_reason: Some("Geometry kernel unavailable".into()),
            source: pack.clone(),
        },
        RuleResult {
            rule_id: "ID-001".into(),
            category: "identity".into(),
            description: "GlobalIds are unique".into(),
            severity: Severity::Error,
            status: RuleStatus::Fail,
            elements_checked: 3,
            failures: 1,
            skipped_reason: None,
            source: pack.clone(),
        },
        RuleResult {
            rule_id: "PS-001".into(),
            category: "properties".into(),
            description: "Required psets".into(),
            severity: Severity::Warning,
            status: RuleStatus::Fail,
            elements_checked: 3,
            failures: 2,
            skipped_reason: None,
            source: RuleSource::ProjectConfig,
        },
    ];
    let findings = vec![
        finding("ID-001", Severity::Error, "0aaa", "<b>&", pack.clone()),
        finding(
            "PS-001",
            Severity::Warning,
            "0bbb",
            "Wall 2",
            RuleSource::ProjectConfig,
        ),
        finding(
            "PS-001",
            Severity::Warning,
            "0ccc",
            "Wall 3",
            ids_src.clone(),
        ),
    ];
    let qto = QtoReport {
        group_by: vec!["type".into(), "storey".into()],
        rows: vec![
            QtoRow {
                group: vec!["IfcSlab".into(), "Level 1".into()],
                count: 1,
                volume_m3: 5.0,
                area_m2: 25.0,
                length_m: 5.0,
                weight_kg: 0.0,
                volume_sources: sources(1, 0),
                area_sources: sources(0, 1),
                length_sources: sources(0, 1),
                cost: None,
            },
            QtoRow {
                group: vec!["IfcWall".into(), "Level 1".into()],
                count: 3,
                volume_m3: 4.5,
                area_m2: 30.25,
                length_m: 12.0,
                weight_kg: 0.0,
                volume_sources: sources(2, 1),
                area_sources: sources(3, 0),
                length_sources: sources(3, 0),
                cost: Some(CostLine {
                    rate_code: "W-01".into(),
                    rate_description: "Walls".into(),
                    unit: "m3".into(),
                    rate: 120.0,
                    quantity: 4.5,
                    amount: 540.0,
                    currency: "EUR".into(),
                    source: "rates.csv".into(),
                }),
            },
        ],
        total_count: 4,
        total_volume_m3: 9.5,
        total_area_m2: 55.25,
        total_length_m: 17.0,
        total_weight_kg: 0.0,
        total_cost: Some(540.0),
        currency: Some("EUR".into()),
        elements_without_quantities: 0,
        method: "Grouped by type and storey. Base quantities first.".into(),
        samples: vec![QtoSample {
            global_id: "0aaa".into(),
            ifc_type: "IfcWall".into(),
            name: Some("<b>&".into()),
            storey: Some("Level 1".into()),
            volume_m3: Some(1.5),
            volume_source: QuantitySource::BaseQuantity,
            area_m2: Some(10.0),
            area_source: QuantitySource::Geometry,
            length_m: None,
            length_source: QuantitySource::None,
            quantity_sets: vec!["Qto_WallBaseQuantities".into()],
        }],
    };
    let ids = IdsReport {
        ids_file: "requirements.ids".into(),
        title: Some("Fire safety".into()),
        specifications_total: 1,
        specifications_passed: 0,
        specifications: vec![IdsSpecificationResult {
            name: "Walls".into(),
            applicable_count: 3,
            passed: false,
            requirements: vec![IdsRequirementResult {
                description: "FireRating shall be provided in Pset_WallCommon".into(),
                facet: "property".into(),
                passed: false,
                failed_entities: vec![IdsFailedEntity {
                    global_id: "0ccc".into(),
                    ifc_type: "IfcWall".into(),
                    name: Some("Wall 3".into()),
                    reason: "property FireRating is missing".into(),
                }],
            }],
            ..Default::default()
        }],
    };
    RunReport {
        tool: ToolInfo::default(),
        model: ModelInfo {
            file_name: "house.ifc".into(),
            sha256: "abc123".into(),
            schema: "IFC4".into(),
            element_count: 4,
            storeys: vec!["Level 1".into()],
            disciplines: vec!["Architecture".into()],
            authoring_tool: Some("SomeCAD 1.0".into()),
            ..Default::default()
        },
        config: Config::default().to_json_value(),
        summary: Summary {
            score: 67,
            verdict: Some(Verdict::Fail),
            elements_checked: 3,
            element_checks: 6,
            errors: 1,
            warnings: 2,
            infos: 0,
            rules_total: 3,
            rules_passed: 0,
            rules_failed: 2,
            rules_skipped: 1,
            rules_not_applicable: 0,
            verdict_rule: verimodel_core::score::VERDICT_RULE.to_string(),
        },
        rules,
        findings,
        qto,
        ids: Some(ids),
        clash: ClashReport {
            enabled: false,
            implemented: false,
            message: "Clash detection is disabled.".into(),
            clashes: vec![],
        },
    }
}

fn read(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn write_all_produces_every_file_deterministically() {
    let report = sample_report();
    let cfg = Config::default();
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();

    let files = report::write_all(&report, &cfg, &a).unwrap();
    let kinds: Vec<&str> = files.iter().map(|f| f.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "report_json",
            "report_html",
            "failures_csv",
            "qto_csv",
            "qto_xlsx",
            "qto_json"
        ]
    );
    for f in &files {
        assert!(f.bytes > 0, "{} is empty", f.name);
        assert_eq!(std::fs::metadata(a.join(&f.name)).unwrap().len(), f.bytes);
    }
    report::write_all(&report, &cfg, &b).unwrap();
    for f in &files {
        assert_eq!(
            read(&a, &f.name),
            read(&b, &f.name),
            "{} differs between two writes",
            f.name
        );
    }

    // failures.csv: header + one line per finding, quoting handled by the csv crate.
    let failures = String::from_utf8(read(&a, "failures.csv")).unwrap();
    let mut rdr = csv::Reader::from_reader(failures.as_bytes());
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(String::from).collect();
    assert_eq!(
        headers,
        vec![
            "severity",
            "rule_id",
            "global_id",
            "step_id",
            "ifc_type",
            "name",
            "storey",
            "message",
            "expected",
            "actual",
            "fix_hint",
            "source"
        ]
    );
    let recs: Vec<csv::StringRecord> = rdr.records().map(|r| r.unwrap()).collect();
    assert_eq!(recs.len(), report.findings.len());
    assert_eq!(failures.lines().count(), report.findings.len() + 1);
    assert_eq!(&recs[0][5], "<b>&");
    assert_eq!(&recs[0][11], "pack:base");
    assert_eq!(&recs[1][11], "project_config");
    assert_eq!(&recs[2][11], "ids:Walls/FireRating");
    assert_eq!(&recs[0][9], "\"nothing\"");

    // qto.csv: header, two rows, TOTAL.
    let qto = String::from_utf8(read(&a, "qto.csv")).unwrap();
    let lines: Vec<&str> = qto.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[0]
        .starts_with("type,storey,count,volume_m3,area_m2,length_m,weight_kg,volume_source"));
    assert!(lines[2].contains("base_quantity=2;geometry=1"));
    assert!(lines[2].contains("W-01,120,m3,540,EUR"));
    assert!(lines[3].starts_with("TOTAL,,4,9.5,55.25,17,0"));
    assert!(lines[3].ends_with("540,EUR"));

    // qto.json is the QtoReport.
    let q: QtoReport = serde_json::from_slice(&read(&a, "qto.json")).unwrap();
    assert_eq!(q, report.qto);

    // xlsx is a zip whose creation date is pinned (otherwise it would embed "now").
    let xlsx = read(&a, "qto.xlsx");
    assert_eq!(&xlsx[..2], b"PK");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(xlsx)).unwrap();
    let mut core = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("docProps/core.xml").unwrap(), &mut core)
        .unwrap();
    assert!(core.contains("2000-01-01T00:00:00Z"), "{core}");
    let names: Vec<String> = zip.file_names().map(String::from).collect();
    assert!(
        names.iter().any(|n| n == "xl/worksheets/sheet3.xml"),
        "three sheets expected: {names:?}"
    );
}

#[test]
fn html_report_contains_escaped_data_and_all_sections() {
    let report = sample_report();
    let strings = Strings::for_language(Language::En);
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("report.html");
    report::write_report_html(&report, &strings, &p).unwrap();
    let html = std::fs::read_to_string(&p).unwrap();

    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("&lt;b&gt;&amp;"), "name must be escaped");
    assert!(
        !html.contains("<b>&"),
        "raw user data must never reach the HTML"
    );
    assert!(html.contains(&strings.get("verdict.FAIL")));
    assert!(html.contains("b-fail"));
    // Verdict rule text (escaped form of the constant).
    assert!(html.contains("FAIL = at least one ERROR."));
    for key in [
        "report.subtitle",
        "report.skipped_note",
        "report.method",
        "report.samples",
        "report.ids",
        "report.bcf_link",
    ] {
        assert!(
            html.contains(&html_escape::encode_text(&strings.get(key)).to_string()),
            "missing {key}"
        );
    }
    assert!(html.contains("Geometry kernel unavailable"));
    assert!(html.contains("FireRating shall be provided in Pset_WallCommon"));
    assert!(html.contains("W-01"));
    assert!(html.contains("540.00 EUR"));
    assert!(html.contains(&strings.get("report.source.base_quantity")));
    assert!(html.contains("Clash detection is disabled."));
    assert!(html.contains(&format!("VeriModel {}", verimodel_core::VERSION)));
    assert!(html.contains("abc123"));
    // Self-contained: no external resources.
    assert!(!html.contains("src=\"http"));
    assert!(!html.contains("href=\"http"));
    assert!(!html.contains("@import"));

    // Spanish labels.
    let es = Strings::for_language(Language::Es);
    let html_es = report::render_report_html(&report, &es);
    assert!(html_es.contains(&es.get("report.summary")));
    assert_ne!(es.get("report.summary"), strings.get("report.summary"));
}

#[test]
fn html_truncates_long_failure_lists() {
    let mut report = sample_report();
    let base = report.findings[0].clone();
    report.findings = (0..report::MAX_HTML_FINDINGS + 5)
        .map(|i| Finding {
            global_id: Some(format!("G{i:05}")),
            ..base.clone()
        })
        .collect();
    let strings = Strings::for_language(Language::En);
    let html = report::render_report_html(&report, &strings);
    assert_eq!(
        html.matches("<tr data-sev=").count(),
        report::MAX_HTML_FINDINGS
    );
    assert!(html.contains("The other 5 are listed in failures.csv"));
    assert!(html.contains("G01999"));
    assert!(!html.contains("G02000"));
}

#[test]
fn empty_report_still_renders() {
    let report = RunReport::default();
    let tmp = tempfile::tempdir().unwrap();
    let files = report::write_all(&report, &Config::default(), tmp.path()).unwrap();
    assert_eq!(files.len(), 6);
    let html = std::fs::read_to_string(tmp.path().join("report.html")).unwrap();
    let strings = Strings::for_language(Language::En);
    assert!(html.contains(&strings.get("report.no_failures")));
    assert!(
        html.contains(&html_escape::encode_text(&strings.get("report.no_quantities")).to_string())
    );
    let failures = std::fs::read_to_string(tmp.path().join("failures.csv")).unwrap();
    assert_eq!(failures.lines().count(), 1);
}
