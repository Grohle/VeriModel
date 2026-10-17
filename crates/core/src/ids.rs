//! IDS (Information Delivery Specification) integration.
//!
//! The IDS engine (IfcTester, driven by the reader) returns facts in
//! [`IdsResults`]. This module turns them into rules and findings so IDS
//! requirements show up in the same tables as rule-pack checks, quoting the
//! requirement verbatim.
//!
//! * [`evaluate`] produces one [`RuleResult`] per (specification, requirement)
//!   pair with rule id `IDS-<spec#>-<req#>` (1-based, two digits), and one
//!   [`Finding`] per failed entity whose `expected` field quotes the
//!   requirement text. Severity: ERROR for every specification. In IDS 1.0 the
//!   cardinality (`required` / `optional` / `prohibited`) only governs whether
//!   applicable entities must, may or must not exist; a requirement that fails
//!   on an applicable entity is always a failure.
//!   A specification without requirements becomes a single rule
//!   `IDS-<spec#>-00` so its verdict is never lost.
//! * [`read_metadata`] reads the `.ids` XML header (title, author, version,
//!   date) and counts the specifications, matching local names so any
//!   namespace prefix works.

use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::error::{CoreError, CoreResult};
use crate::model::{
    Finding, IdsReport, IdsRequirementResult, IdsResults, IdsSpecificationResult, RuleResult,
    RuleSource, RuleStatus, Severity,
};

/// Rules + findings derived from an IDS audit.
#[derive(Debug, Clone, Default)]
pub struct IdsEvaluation {
    pub rules: Vec<RuleResult>,
    pub findings: Vec<Finding>,
    pub report: IdsReport,
}

/// Metadata from the IDS file header.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IdsMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub date: Option<String>,
    pub specification_count: usize,
}

/// Rule category of every IDS-derived rule.
pub const IDS_CATEGORY: &str = "ids";

/// Severity of the rules derived from a specification. IDS 1.0 cardinality
/// ("required" / "optional" / "prohibited") says whether applicable entities
/// must, may or must not exist; it never downgrades a failed requirement, so
/// every IDS rule is an ERROR.
fn severity_for(cardinality: Option<&str>) -> Severity {
    let _ = cardinality;
    Severity::Error
}

fn status_for(passed: bool, applicable_count: usize) -> RuleStatus {
    match (passed, applicable_count) {
        (true, 0) => RuleStatus::NotApplicable,
        (true, _) => RuleStatus::Pass,
        (false, _) => RuleStatus::Fail,
    }
}

fn fix_hint_for(spec: &IdsSpecificationResult) -> String {
    spec.instructions
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| format!("See IDS specification '{}'.", spec.name))
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Text describing a specification that has no requirements (its applicability).
fn applicability_text(spec: &IdsSpecificationResult) -> String {
    let parts: Vec<&str> = spec
        .applicability
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        spec.description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("applicability")
            .to_string()
    } else {
        parts.join("; ")
    }
}

/// Message of a model-level finding, used when a requirement failed but the
/// IDS engine named no failing entity (e.g. a required specification with
/// no applicable element at all).
fn model_level_message(spec: &IdsSpecificationResult, requirement_text: &str) -> String {
    if spec.applicable_count == 0 {
        let card = spec.cardinality.as_deref().unwrap_or("required");
        format!(
            "Specification '{}' is {card} but no applicable element was found",
            spec.name
        )
    } else {
        format!("Requirement not met: {requirement_text}")
    }
}

/// Rule + findings of one (specification, requirement) pair.
fn evaluate_requirement(
    rule_id: String,
    spec: &IdsSpecificationResult,
    requirement_text: &str,
    passed: bool,
    failed: &[crate::model::IdsFailedEntity],
) -> (RuleResult, Vec<Finding>) {
    let severity = severity_for(spec.cardinality.as_deref());
    let source = RuleSource::Ids {
        specification: spec.name.clone(),
        requirement: requirement_text.to_string(),
    };
    let fix_hint = fix_hint_for(spec);
    let mut findings: Vec<Finding> = failed
        .iter()
        .map(|fe| {
            let reason = non_empty(&fe.reason);
            Finding {
                rule_id: rule_id.clone(),
                severity,
                global_id: non_empty(&fe.global_id),
                step_id: None,
                ifc_type: non_empty(&fe.ifc_type),
                name: fe.name.as_deref().and_then(non_empty),
                storey: None,
                message: reason
                    .clone()
                    .unwrap_or_else(|| requirement_text.to_string()),
                expected: Some(requirement_text.to_string()),
                actual: reason,
                fix_hint: Some(fix_hint.clone()),
                source: source.clone(),
            }
        })
        .collect();
    if !passed && findings.is_empty() {
        findings.push(Finding {
            rule_id: rule_id.clone(),
            severity,
            global_id: None,
            step_id: None,
            ifc_type: None,
            name: None,
            storey: None,
            message: model_level_message(spec, requirement_text),
            expected: Some(requirement_text.to_string()),
            actual: None,
            fix_hint: Some(fix_hint.clone()),
            source: source.clone(),
        });
    }
    findings.sort_by(|a, b| {
        a.global_id
            .cmp(&b.global_id)
            .then_with(|| a.message.cmp(&b.message))
    });

    let rule = RuleResult {
        rule_id,
        category: IDS_CATEGORY.to_string(),
        description: format!("{}: {}", spec.name, requirement_text),
        severity,
        status: status_for(passed, spec.applicable_count),
        elements_checked: spec.applicable_count,
        failures: findings.len(),
        skipped_reason: None,
        source,
    };
    (rule, findings)
}

/// Convert IDS engine results into rules and findings.
pub fn evaluate(results: &IdsResults) -> IdsEvaluation {
    let mut rules = Vec::new();
    let mut findings = Vec::new();
    for (i, spec) in results.specifications.iter().enumerate() {
        if spec.requirements.is_empty() {
            // No requirement: the specification's own verdict (e.g. a prohibited
            // applicability) is the only thing to report.
            let text = applicability_text(spec);
            let (r, f) = evaluate_requirement(
                format!("IDS-{:02}-00", i + 1),
                spec,
                &text,
                spec.passed,
                &[],
            );
            rules.push(r);
            findings.extend(f);
            continue;
        }
        for (j, req) in spec.requirements.iter().enumerate() {
            let IdsRequirementResult {
                description,
                passed,
                failed_entities,
                ..
            } = req;
            let (r, f) = evaluate_requirement(
                format!("IDS-{:02}-{:02}", i + 1, j + 1),
                spec,
                description,
                *passed,
                failed_entities,
            );
            rules.push(r);
            findings.extend(f);
        }
    }
    let report = IdsReport {
        ids_file: results.ids_file.clone(),
        title: results.title.clone(),
        specifications_total: results.specifications.len(),
        specifications_passed: results.specifications.iter().filter(|s| s.passed).count(),
        specifications: results.specifications.clone(),
    };
    IdsEvaluation {
        rules,
        findings,
        report,
    }
}

/// Parse the header of an `.ids` file.
pub fn read_metadata(path: &Path) -> CoreResult<IdsMetadata> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| CoreError::Io(format!("cannot read IDS file {}: {e}", path.display())))?;
    parse_metadata(&text).map_err(|msg| {
        CoreError::Config(format!("IDS file {} is not valid: {msg}", path.display()))
    })
}

/// Parse IDS 1.0 metadata from XML text. Element names are matched on their
/// local name so `ids:info`, `info` or any other prefix are all accepted.
fn parse_metadata(xml: &str) -> Result<IdsMetadata, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut meta = IdsMetadata::default();
    // Local names of the currently open elements.
    let mut stack: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut root_seen = false;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("XML error at byte {}: {e}", reader.buffer_position()))?;
        match event {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if stack.is_empty() {
                    if root_seen || name != "ids" {
                        return Err(format!("root element is '{name}', expected 'ids'"));
                    }
                    root_seen = true;
                }
                if is_specification(&stack, &name) {
                    meta.specification_count += 1;
                }
                stack.push(name);
                text.clear();
            }
            Event::Empty(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if stack.is_empty() {
                    return Err(format!(
                        "root element is '{name}' and empty, expected an 'ids' document"
                    ));
                }
                if is_specification(&stack, &name) {
                    meta.specification_count += 1;
                }
            }
            Event::Text(t) => {
                let s = t.unescape().map_err(|e| format!("XML error: {e}"))?;
                text.push_str(&s);
            }
            Event::CData(c) => text.push_str(&String::from_utf8_lossy(&c.into_inner())),
            Event::End(_) => {
                let name = stack.pop().unwrap_or_default();
                if stack.len() == 2 && stack[0] == "ids" && stack[1] == "info" {
                    let value = non_empty(&text);
                    match name.as_str() {
                        "title" => meta.title = value,
                        "author" => meta.author = value,
                        "version" => meta.version = value,
                        "date" => meta.date = value,
                        _ => {}
                    }
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen {
        return Err("no 'ids' root element found".into());
    }
    if !stack.is_empty() {
        return Err(format!(
            "the document ends before '{}' is closed",
            stack.last().map(String::as_str).unwrap_or("")
        ));
    }
    Ok(meta)
}

/// True when an element named `name`, opened under `stack`, is `ids/specifications/specification`.
fn is_specification(stack: &[String], name: &str) -> bool {
    name == "specification" && stack.len() == 2 && stack[0] == "ids" && stack[1] == "specifications"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::IdsFailedEntity;

    const SAMPLE_IDS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ids:ids xmlns:ids="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://standards.buildingsmart.org/IDS http://standards.buildingsmart.org/IDS/1.0/ids.xsd">
  <ids:info>
    <ids:title>Fire safety &amp; walls</ids:title>
    <ids:copyright>ACME</ids:copyright>
    <ids:version>1.2</ids:version>
    <ids:description>Demo requirements</ids:description>
    <ids:author>bim@example.com</ids:author>
    <ids:date>2025-03-01</ids:date>
    <ids:purpose>QA</ids:purpose>
    <ids:milestone>Design</ids:milestone>
  </ids:info>
  <ids:specifications>
    <ids:specification name="Walls have a fire rating" ifcVersion="IFC4" description="Every wall">
      <ids:applicability minOccurs="1" maxOccurs="unbounded">
        <ids:entity><ids:name><ids:simpleValue>IFCWALL</ids:simpleValue></ids:name></ids:entity>
      </ids:applicability>
      <ids:requirements>
        <ids:property dataType="IFCLABEL">
          <ids:propertySet><ids:simpleValue>Pset_WallCommon</ids:simpleValue></ids:propertySet>
          <ids:baseName><ids:simpleValue>FireRating</ids:simpleValue></ids:baseName>
        </ids:property>
      </ids:requirements>
    </ids:specification>
    <ids:specification name="Doors are named" ifcVersion="IFC4">
      <ids:applicability minOccurs="0" maxOccurs="unbounded">
        <ids:entity><ids:name><ids:simpleValue>IFCDOOR</ids:simpleValue></ids:name></ids:entity>
      </ids:applicability>
      <ids:requirements>
        <ids:attribute><ids:name><ids:simpleValue>Name</ids:simpleValue></ids:name></ids:attribute>
      </ids:requirements>
    </ids:specification>
  </ids:specifications>
</ids:ids>"#;

    #[test]
    fn reads_prefixed_metadata() {
        let m = parse_metadata(SAMPLE_IDS).unwrap();
        assert_eq!(
            m,
            IdsMetadata {
                title: Some("Fire safety & walls".into()),
                author: Some("bim@example.com".into()),
                version: Some("1.2".into()),
                date: Some("2025-03-01".into()),
                specification_count: 2,
            }
        );
    }

    #[test]
    fn reads_unprefixed_metadata_from_file() {
        let xml = SAMPLE_IDS
            .replace("ids:", "")
            .replace("xmlns:ids=", "xmlns=");
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.ids");
        std::fs::write(&p, xml).unwrap();
        let m = read_metadata(&p).unwrap();
        assert_eq!(m.title.as_deref(), Some("Fire safety & walls"));
        assert_eq!(m.specification_count, 2);
    }

    #[test]
    fn rejects_non_ids_and_broken_xml() {
        assert!(parse_metadata("<foo/>").is_err());
        assert!(parse_metadata("<html><body/></html>").is_err());
        assert!(
            parse_metadata("<ids:ids xmlns:ids=\"x\"><ids:info>").is_err(),
            "truncated document"
        );
        assert!(parse_metadata("<ids><info><title>x</wrong></info></ids>").is_err());
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_metadata(&dir.path().join("missing.ids")),
            Err(CoreError::Io(_))
        ));
        // Missing header fields are simply None.
        let m =
            parse_metadata("<ids><info><title>Only title</title></info><specifications/></ids>")
                .unwrap();
        assert_eq!(m.title.as_deref(), Some("Only title"));
        assert_eq!(m.author, None);
        assert_eq!(m.specification_count, 0);
    }

    fn results() -> IdsResults {
        IdsResults {
            ids_file: "fire.ids".into(),
            title: Some("Fire safety".into()),
            specifications: vec![
                IdsSpecificationResult {
                    name: "Walls have a fire rating".into(),
                    instructions: Some(
                        "Fill Pset_WallCommon.FireRating in the authoring tool.".into(),
                    ),
                    cardinality: Some("required".into()),
                    applicable_count: 3,
                    passed: false,
                    requirements: vec![
                        IdsRequirementResult {
                            description:
                                "FireRating data shall be provided in the dataset Pset_WallCommon"
                                    .into(),
                            facet: "property".into(),
                            passed: false,
                            failed_entities: vec![
                                IdsFailedEntity {
                                    global_id: "2O2Fr$t4X7Zf8NOew3FLOH".into(),
                                    ifc_type: "IfcWall".into(),
                                    name: Some("W-02".into()),
                                    reason:
                                        "The property set does not contain the required property"
                                            .into(),
                                },
                                IdsFailedEntity {
                                    global_id: "1hOSvn6df7F8_7GcBWlRGQ".into(),
                                    ifc_type: "IfcWall".into(),
                                    name: None,
                                    reason: String::new(),
                                },
                            ],
                        },
                        IdsRequirementResult {
                            description: "Name shall be provided".into(),
                            facet: "attribute".into(),
                            passed: true,
                            failed_entities: vec![],
                        },
                    ],
                    ..Default::default()
                },
                IdsSpecificationResult {
                    name: "Doors are named".into(),
                    cardinality: Some("optional".into()),
                    applicable_count: 0,
                    passed: true,
                    requirements: vec![IdsRequirementResult {
                        description: "Name shall be provided".into(),
                        passed: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                IdsSpecificationResult {
                    name: "No proxies".into(),
                    cardinality: Some("prohibited".into()),
                    applicable_count: 1,
                    applicability: vec!["All IfcBuildingElementProxy data".into()],
                    passed: false,
                    requirements: vec![],
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn evaluate_builds_rules_and_findings() {
        let ev = evaluate(&results());
        let ids: Vec<&str> = ev.rules.iter().map(|r| r.rule_id.as_str()).collect();
        assert_eq!(ids, ["IDS-01-01", "IDS-01-02", "IDS-02-01", "IDS-03-00"]);

        let r = &ev.rules[0];
        assert_eq!(r.category, "ids");
        assert_eq!(r.description, "Walls have a fire rating: FireRating data shall be provided in the dataset Pset_WallCommon");
        assert_eq!(r.severity, Severity::Error);
        assert_eq!(r.status, RuleStatus::Fail);
        assert_eq!(r.elements_checked, 3);
        assert_eq!(r.failures, 2);
        assert_eq!(
            r.source,
            RuleSource::Ids {
                specification: "Walls have a fire rating".into(),
                requirement: "FireRating data shall be provided in the dataset Pset_WallCommon"
                    .into()
            }
        );
        assert_eq!(ev.rules[1].status, RuleStatus::Pass);
        assert_eq!(ev.rules[2].status, RuleStatus::NotApplicable);
        assert_eq!(
            ev.rules[2].severity,
            Severity::Error,
            "optional specs still fail as errors"
        );
        assert_eq!(
            ev.rules[3].severity,
            Severity::Error,
            "prohibited specs fail as errors"
        );
        assert_eq!(ev.rules[3].status, RuleStatus::Fail);

        // Findings: two failed walls (sorted by GlobalId) + one model-level finding for the prohibited spec.
        assert_eq!(ev.findings.len(), 3);
        let f0 = &ev.findings[0];
        assert_eq!(f0.global_id.as_deref(), Some("1hOSvn6df7F8_7GcBWlRGQ"));
        assert_eq!(
            f0.message, "FireRating data shall be provided in the dataset Pset_WallCommon",
            "empty reason falls back to the requirement"
        );
        assert_eq!(f0.actual, None);
        let f1 = &ev.findings[1];
        assert_eq!(f1.name.as_deref(), Some("W-02"));
        assert_eq!(f1.ifc_type.as_deref(), Some("IfcWall"));
        assert_eq!(
            f1.message,
            "The property set does not contain the required property"
        );
        assert_eq!(
            f1.expected.as_deref(),
            Some("FireRating data shall be provided in the dataset Pset_WallCommon")
        );
        assert_eq!(
            f1.actual.as_deref(),
            Some("The property set does not contain the required property")
        );
        assert_eq!(
            f1.fix_hint.as_deref(),
            Some("Fill Pset_WallCommon.FireRating in the authoring tool.")
        );
        let f2 = &ev.findings[2];
        assert_eq!(f2.rule_id, "IDS-03-00");
        assert_eq!(f2.global_id, None);
        assert_eq!(
            f2.fix_hint.as_deref(),
            Some("See IDS specification 'No proxies'.")
        );
        assert_eq!(
            f2.expected.as_deref(),
            Some("All IfcBuildingElementProxy data")
        );

        assert_eq!(ev.report.ids_file, "fire.ids");
        assert_eq!(ev.report.title.as_deref(), Some("Fire safety"));
        assert_eq!(ev.report.specifications_total, 3);
        assert_eq!(ev.report.specifications_passed, 1);
        assert_eq!(ev.report.specifications.len(), 3);

        // Deterministic.
        let again = evaluate(&results());
        assert_eq!(ev.rules, again.rules);
        assert_eq!(ev.findings, again.findings);
    }

    #[test]
    fn failed_requirement_without_entities_is_reported() {
        let res = IdsResults {
            ids_file: "x.ids".into(),
            specifications: vec![IdsSpecificationResult {
                name: "Spaces exist".into(),
                cardinality: None,
                applicable_count: 0,
                passed: false,
                requirements: vec![IdsRequirementResult {
                    description: "Name shall be provided".into(),
                    passed: false,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let ev = evaluate(&res);
        assert_eq!(ev.rules[0].status, RuleStatus::Fail);
        assert_eq!(ev.rules[0].severity, Severity::Error);
        assert_eq!(ev.rules[0].failures, 1);
        assert_eq!(ev.findings.len(), 1);
        assert!(
            ev.findings[0].message.contains("no applicable element"),
            "{}",
            ev.findings[0].message
        );
    }

    #[test]
    fn empty_results() {
        let ev = evaluate(&IdsResults::default());
        assert!(ev.rules.is_empty());
        assert!(ev.findings.is_empty());
        assert_eq!(ev.report.specifications_total, 0);
    }
}
