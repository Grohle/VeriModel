//! Integration: IDS results -> rules/findings -> BCF topics and issues.csv.

use std::io::Read;

use verimodel_core::bcf::{write_bcfzip, write_issues_csv};
use verimodel_core::config::Config;
use verimodel_core::ids::evaluate;
use verimodel_core::model::{
    Element, IdsFailedEntity, IdsRequirementResult, IdsResults, IdsSpecificationResult, Model,
    ModelInfo, RunReport,
};

fn ids_results() -> IdsResults {
    IdsResults {
        ids_file: "fire.ids".into(),
        title: Some("Fire".into()),
        specifications: vec![IdsSpecificationResult {
            name: "Walls have a fire rating".into(),
            cardinality: Some("required".into()),
            applicable_count: 2,
            passed: false,
            requirements: vec![IdsRequirementResult {
                description: "FireRating shall be provided in Pset_WallCommon".into(),
                facet: "property".into(),
                passed: false,
                failed_entities: vec![IdsFailedEntity {
                    global_id: "2O2Fr$t4X7Zf8NOew3FLOH".into(),
                    ifc_type: "IfcWall".into(),
                    name: Some("W-01".into()),
                    reason: "FireRating is missing".into(),
                }],
            }],
            ..Default::default()
        }],
    }
}

#[test]
fn ids_failures_become_bcf_topics() {
    let ev = evaluate(&ids_results());
    let model = Model {
        info: ModelInfo {
            file_name: "m.ifc".into(),
            sha256: "cd".repeat(32),
            ..Default::default()
        },
        elements: vec![Element {
            global_id: "2O2Fr$t4X7Zf8NOew3FLOH".into(),
            ifc_type: "IfcWall".into(),
            ..Default::default()
        }],
        ids_results: Some(ids_results()),
    };
    let mut report = RunReport {
        model: model.info.clone(),
        config: Config::default().to_json_value(),
        rules: ev.rules,
        findings: ev.findings,
        ids: Some(ev.report),
        ..Default::default()
    };
    report.normalise();

    let dir = tempfile::tempdir().unwrap();
    let bcf_path = dir.path().join("issues.bcfzip");
    let stats = write_bcfzip(&report, &model, &Config::default(), &bcf_path).unwrap();
    assert_eq!(stats.topics, 1);
    assert_eq!(stats.with_viewpoint, 1);
    assert_eq!(stats.version, "2.1");

    let mut zip = zip::ZipArchive::new(std::fs::File::open(&bcf_path).unwrap()).unwrap();
    let markup_name = zip
        .file_names()
        .find(|n| n.ends_with("/markup.bcf"))
        .unwrap()
        .to_string();
    let mut markup = String::new();
    zip.by_name(&markup_name)
        .unwrap()
        .read_to_string(&mut markup)
        .unwrap();
    assert!(
        markup.contains("<Title>[IDS-01-01] FireRating is missing</Title>"),
        "{markup}"
    );
    assert!(markup
        .contains("Walls have a fire rating: FireRating shall be provided in Pset_WallCommon"));
    assert!(markup.contains("<Labels>ids</Labels>"));

    let csv_path = dir.path().join("issues.csv");
    assert_eq!(write_issues_csv(&report, &csv_path).unwrap(), 1);
    let csv_text = std::fs::read_to_string(&csv_path).unwrap();
    let guid = markup_name.trim_end_matches("/markup.bcf");
    assert!(csv_text.contains(guid), "issues.csv shares the topic GUID");
}
