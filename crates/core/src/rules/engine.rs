//! Rule evaluation: scope resolution, config-generated rules, IDS merge and scoring.

use std::collections::BTreeSet;
use std::path::Path;

use regex::Regex;

use crate::config::{Config, ElementFilter, RequiredPset};
use crate::error::{CoreError, CoreResult};
use crate::model::{Element, Finding, Model, RuleResult, RuleSource, RuleStatus, Summary};
use crate::rules::checks::{CheckContext, CheckKind, CheckOutcome};
use crate::rules::pack::{Rule, RulePack, RuleScope};
use crate::score;

/// Output of the rule engine for one model.
#[derive(Debug, Clone, Default)]
pub struct Evaluation {
    pub rules: Vec<RuleResult>,
    pub findings: Vec<Finding>,
    pub summary: Summary,
}

/// Does `e` satisfy a config element filter?
pub fn filter_matches(e: &Element, f: &ElementFilter) -> bool {
    if let Some(t) = &f.ifc_type {
        if !t.is_empty() && !e.is_a(t) {
            return false;
        }
    }
    if let Some(p) = &f.name_pattern {
        if let Ok(re) = Regex::new(p) {
            if !re.is_match(e.name.as_deref().unwrap_or("")) {
                return false;
            }
        }
    }
    if let Some(s) = &f.storey {
        if e.storey.as_deref() != Some(s.as_str()) {
            return false;
        }
    }
    for (k, v) in &f.property_equals {
        let (pset, prop) = match k.split_once('.') {
            Some((p, n)) => (Some(p), n),
            None => (None, k.as_str()),
        };
        match e.property(pset, prop) {
            Some(val) if val.display() == *v => {}
            _ => return false,
        }
    }
    true
}

/// Indices of the elements a scope selects.
pub fn resolve_scope(model: &Model, scope: &RuleScope) -> Vec<usize> {
    model
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            if !scope.include_spatial && e.is_spatial() && scope.ifc_types.is_empty() {
                return false;
            }
            if !scope.ifc_types.is_empty() && !scope.ifc_types.iter().any(|t| e.is_a(t)) {
                return false;
            }
            if scope.exclude_types.iter().any(|t| e.is_a(t)) {
                return false;
            }
            if let Some(f) = &scope.filter {
                if !filter_matches(e, f) {
                    return false;
                }
            }
            true
        })
        .map(|(i, _)| i)
        .collect()
}

/// Rules generated from `requirements.required_psets` in project.yaml.
pub fn rules_from_config(config: &Config) -> Vec<Rule> {
    let mut out = Vec::new();
    for (n, rp) in config.requirements.required_psets.iter().enumerate() {
        out.extend(rules_for_required_pset(n, rp));
    }
    out
}

fn rules_for_required_pset(n: usize, rp: &RequiredPset) -> Vec<Rule> {
    let scope = RuleScope { ifc_types: rp.applies_to.clone(), ..Default::default() };
    let severity = rp.severity.clone().unwrap_or_else(|| "error".into());
    let mut rules = vec![Rule {
        id: format!("CFG-PSET-{:03}", n + 1),
        category: "properties".into(),
        description: format!("Property set {} must exist on {}", rp.pset, applies_label(&rp.applies_to)),
        severity: severity.clone(),
        enabled: true,
        enabled_if: None,
        scope: scope.clone(),
        check: CheckKind::PsetExists { pset: rp.pset.clone() },
        fix_hint: Some(format!("Add the property set {} to the element in the authoring tool.", rp.pset)),
    }];
    for (m, prop) in rp.properties.iter().enumerate() {
        rules.push(Rule {
            id: format!("CFG-PSET-{:03}-{:02}", n + 1, m + 1),
            category: "properties".into(),
            description: format!("Property {}.{} must exist and have a value on {}", rp.pset, prop, applies_label(&rp.applies_to)),
            severity: severity.clone(),
            enabled: true,
            enabled_if: None,
            scope: scope.clone(),
            check: CheckKind::PropertyExists {
                pset: Some(rp.pset.clone()),
                property: prop.clone(),
                require_value: true,
                data_type: None,
            },
            fix_hint: Some(format!("Fill in {}.{} on the element.", rp.pset, prop)),
        });
    }
    rules
}

fn applies_label(types: &[String]) -> String {
    if types.is_empty() {
        "all elements".into()
    } else {
        types.join(", ")
    }
}

/// Allowed classification codes from config (inline list + optional file).
pub fn load_allowed_codes(config: &Config, config_dir: Option<&Path>) -> CoreResult<BTreeSet<String>> {
    let mut set: BTreeSet<String> = config.classification.allowed_codes.iter().map(|s| s.trim().to_string()).collect();
    if let Some(f) = &config.classification.allowed_codes_file {
        let path = Config::resolve_path(config_dir, f);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| CoreError::Config(format!("cannot read classification.allowed_codes_file {}: {e}", path.display())))?;
        let mut lines = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#'));
        if let Some(first) = lines.next() {
            let header: Vec<&str> = first.split(',').map(|s| s.trim().to_ascii_lowercase()).map(|s| Box::leak(s.into_boxed_str()) as &str).collect();
            let code_col = header.iter().position(|h| *h == "code");
            match code_col {
                Some(col) => {
                    for l in lines {
                        if let Some(c) = l.split(',').nth(col) {
                            set.insert(c.trim().trim_matches('"').to_string());
                        }
                    }
                }
                None => {
                    set.insert(first.split(',').next().unwrap_or("").trim().to_string());
                    for l in lines {
                        set.insert(l.split(',').next().unwrap_or("").trim().to_string());
                    }
                }
            }
        }
    }
    set.remove("");
    Ok(set)
}

/// Evaluate all rules (pack + config generated) against the model.
pub fn evaluate(model: &Model, config: &Config, pack: &RulePack, config_dir: Option<&Path>) -> CoreResult<Evaluation> {
    let allowed_codes = load_allowed_codes(config, config_dir)?;
    let mut rules: Vec<(Rule, RuleSource)> = pack
        .rules
        .iter()
        .cloned()
        .map(|r| (r, RuleSource::RulePack { pack: pack.name.clone() }))
        .collect();
    rules.extend(rules_from_config(config).into_iter().map(|r| (r, RuleSource::ProjectConfig)));

    let mut results = Vec::new();
    let mut findings = Vec::new();

    for (rule, source) in &rules {
        if !rule.enabled || !config.checks.is_enabled(&rule.id, &rule.category) {
            continue;
        }
        if let Some(cond) = &rule.enabled_if {
            let v = config.lookup(cond.trim_start_matches("$config.")).unwrap_or(serde_json::Value::Bool(false));
            if !v.as_bool().unwrap_or(false) {
                continue;
            }
        }
        let scope = resolve_scope(model, &rule.scope);
        let severity = rule.severity();
        let ctx = CheckContext { model, config, scope: &scope, allowed_codes: &allowed_codes };
        let checked = if rule.check.is_model_level() { scope.len().max(1) } else { scope.len() };
        let outcome = rule.check.run(&ctx)?;
        match outcome {
            CheckOutcome::Skipped { reason } => results.push(RuleResult {
                rule_id: rule.id.clone(),
                category: rule.category.clone(),
                description: rule.description.clone(),
                severity,
                status: RuleStatus::Skipped,
                elements_checked: 0,
                failures: 0,
                skipped_reason: Some(reason),
                source: source.clone(),
            }),
            CheckOutcome::Ran { failures } => {
                let status = if failures.is_empty() {
                    if checked == 0 {
                        RuleStatus::NotApplicable
                    } else {
                        RuleStatus::Pass
                    }
                } else {
                    RuleStatus::Fail
                };
                results.push(RuleResult {
                    rule_id: rule.id.clone(),
                    category: rule.category.clone(),
                    description: rule.description.clone(),
                    severity,
                    status,
                    elements_checked: checked,
                    failures: failures.len(),
                    skipped_reason: None,
                    source: source.clone(),
                });
                for f in failures {
                    let e = f.element.map(|i| &model.elements[i]);
                    findings.push(Finding {
                        rule_id: rule.id.clone(),
                        severity,
                        global_id: e.map(|e| e.global_id.clone()),
                        step_id: e.map(|e| e.step_id),
                        ifc_type: e.map(|e| e.ifc_type.clone()),
                        name: e.and_then(|e| e.name.clone()),
                        storey: e.and_then(|e| e.storey.clone()),
                        message: f.message,
                        expected: f.expected,
                        actual: f.actual,
                        fix_hint: rule.fix_hint.clone(),
                        source: source.clone(),
                    });
                }
            }
        }
    }

    results.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
    let summary = score::summarise(&results, &findings);
    Ok(Evaluation { rules: results, findings, summary })
}

/// Merge extra rule results / findings (e.g. from IDS) and recompute the summary.
pub fn merge(eval: &mut Evaluation, rules: Vec<RuleResult>, findings: Vec<Finding>) {
    eval.rules.extend(rules);
    eval.findings.extend(findings);
    eval.rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
    eval.summary = score::summarise(&eval.rules, &eval.findings);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Geometry, ModelInfo, Severity, SpatialNode, SpatialRef};

    fn elem(id: &str, t: &str, name: &str, storey: Option<&str>) -> Element {
        Element {
            global_id: id.into(),
            step_id: 1,
            ifc_type: t.into(),
            ancestors: vec![t.into(), "IfcBuildingElement".into(), "IfcElement".into(), "IfcProduct".into()],
            name: Some(name.into()),
            storey: storey.map(|s| s.to_string()),
            container: storey.map(|s| SpatialRef { global_id: "S1".into(), ifc_type: "IfcBuildingStorey".into(), name: Some(s.into()) }),
            has_representation: true,
            geometry: Some(Geometry { bbox_min: [0.0; 3], bbox_max: [1.0; 3], volume_m3: Some(1.0), area_m2: Some(6.0), triangle_count: 12 }),
            ..Default::default()
        }
    }

    fn model(elements: Vec<Element>) -> Model {
        let mut m = Model {
            info: ModelInfo {
                schema: "IFC4".into(),
                geometry_available: true,
                spatial_tree: vec![SpatialNode {
                    global_id: "SITE".into(),
                    ifc_type: "IfcSite".into(),
                    children: vec![SpatialNode {
                        global_id: "B".into(),
                        ifc_type: "IfcBuilding".into(),
                        children: vec![SpatialNode { global_id: "S1".into(), ifc_type: "IfcBuildingStorey".into(), ..Default::default() }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            elements,
            ids_results: None,
        };
        m.sort_elements();
        m
    }

    #[test]
    fn duplicate_global_id_is_an_error() {
        let m = model(vec![
            elem("0fqX614OH1YO1Njdxms2$Q", "IfcWall", "W1", Some("L1")),
            elem("0fqX614OH1YO1Njdxms2$Q", "IfcWall", "W2", Some("L1")),
            elem("1fqX614OH1YO1Njdxms2$Q", "IfcSlab", "S1", Some("L1")),
        ]);
        let cfg = Config::default();
        let ev = evaluate(&m, &cfg, &RulePack::generic(), None).unwrap();
        let dup: Vec<_> = ev.findings.iter().filter(|f| f.rule_id == "ID-001").collect();
        assert_eq!(dup.len(), 2, "{:?}", ev.findings);
        assert_eq!(dup[0].severity, Severity::Error);
        assert_eq!(ev.summary.verdict, Some(crate::model::Verdict::Fail));
    }

    #[test]
    fn clean_model_passes() {
        let m = model(vec![
            elem("0fqX614OH1YO1Njdxms2$Q", "IfcWall", "W1", Some("L1")),
            elem("1fqX614OH1YO1Njdxms2$Q", "IfcSlab", "S1", Some("L1")),
        ]);
        let cfg = Config::default();
        let ev = evaluate(&m, &cfg, &RulePack::generic(), None).unwrap();
        let errors: Vec<_> = ev.findings.iter().filter(|f| f.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "{:#?}", errors);
        assert_eq!(ev.summary.verdict, Some(crate::model::Verdict::Pass));
        assert_eq!(ev.summary.score, 100);
    }

    #[test]
    fn element_without_storey_fails_spatial_rule() {
        let m = model(vec![elem("0fqX614OH1YO1Njdxms2$Q", "IfcWall", "W1", None)]);
        let ev = evaluate(&m, &Config::default(), &RulePack::generic(), None).unwrap();
        assert!(ev.findings.iter().any(|f| f.rule_id == "SP-001"), "{:?}", ev.findings);
    }

    #[test]
    fn config_required_pset_generates_rule_and_toggles_work() {
        let m = model(vec![elem("0fqX614OH1YO1Njdxms2$Q", "IfcWall", "W1", Some("L1"))]);
        let cfg = Config::from_yaml(
            "requirements:\n  required_psets:\n    - pset: Pset_WallCommon\n      applies_to: [IfcWall]\n      properties: [FireRating]\n",
        )
        .unwrap();
        let ev = evaluate(&m, &cfg, &RulePack::generic(), None).unwrap();
        assert!(ev.findings.iter().any(|f| f.rule_id == "CFG-PSET-001"));
        assert!(ev.findings.iter().any(|f| f.rule_id == "CFG-PSET-001-01"));
        let cfg2 = Config::from_yaml("checks:\n  rules:\n    ID-001: false\n").unwrap();
        let ev2 = evaluate(&m, &cfg2, &RulePack::generic(), None).unwrap();
        assert!(!ev2.rules.iter().any(|r| r.rule_id == "ID-001"));
    }

    #[test]
    fn geometry_rules_are_skipped_without_kernel() {
        let mut m = model(vec![elem("0fqX614OH1YO1Njdxms2$Q", "IfcWall", "W1", Some("L1"))]);
        m.info.geometry_available = false;
        let ev = evaluate(&m, &Config::default(), &RulePack::generic(), None).unwrap();
        let geo = ev.rules.iter().find(|r| r.rule_id == "GEO-002").unwrap();
        assert_eq!(geo.status, RuleStatus::Skipped);
        assert!(geo.skipped_reason.is_some());
        assert!(ev.summary.rules_skipped >= 1);
    }

    #[test]
    fn naming_pattern_from_config() {
        let m = model(vec![elem("0fqX614OH1YO1Njdxms2$Q", "IfcWall", "Wall-A", Some("L1"))]);
        let cfg = Config::from_yaml("naming:\n  patterns:\n    IfcWall: '^W-[0-9]{3}$'\n").unwrap();
        let ev = evaluate(&m, &cfg, &RulePack::generic(), None).unwrap();
        assert!(ev.findings.iter().any(|f| f.rule_id == "NAM-001"), "{:?}", ev.findings);
    }
}
