//! Rule pack format and loading.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::config::{Config, ElementFilter};
use crate::error::{CoreError, CoreResult};
use crate::model::Severity;
use crate::rules::checks::CheckKind;

/// Built-in generic rule pack, shipped as data (not code).
pub const GENERIC_RULE_PACK_YAML: &str = include_str!("../../rule_packs/generic.yaml");

/// A rule pack: a named, versioned list of rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulePack {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub description: Option<String>,
    pub rules: Vec<Rule>,
}

fn default_version() -> u32 {
    1
}

/// Which elements a rule applies to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RuleScope {
    /// Element types (or ancestors) the rule applies to. Empty = all products.
    pub ifc_types: Vec<String>,
    /// Types (or ancestors) excluded from the scope.
    pub exclude_types: Vec<String>,
    /// Include spatial structure elements (sites, buildings, storeys, spaces). Default false.
    pub include_spatial: bool,
    /// Additional filter on name / storey / property values.
    pub filter: Option<ElementFilter>,
}

/// One rule definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Stable identifier, e.g. "ID-001". Must be unique in the pack.
    pub id: String,
    /// Category used for toggles and grouping: identity, spatial, properties, geometry, classification, naming, sanity, ids.
    pub category: String,
    pub description: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional `$config.<path>` boolean that must be true for the rule to run (e.g. `$config.classification.required`).
    #[serde(default)]
    pub enabled_if: Option<String>,
    #[serde(default)]
    pub scope: RuleScope,
    pub check: CheckKind,
    #[serde(default)]
    pub fix_hint: Option<String>,
}

fn default_severity() -> String {
    "error".into()
}
fn default_true() -> bool {
    true
}

impl Rule {
    pub fn severity(&self) -> Severity {
        self.severity.parse().unwrap_or(Severity::Error)
    }
}

impl RulePack {
    /// Parse a rule pack from YAML text and validate it.
    pub fn from_yaml(text: &str) -> CoreResult<RulePack> {
        let pack: RulePack = serde_yaml::from_str(text).map_err(|e| CoreError::RulePack(e.to_string()))?;
        pack.validate()?;
        Ok(pack)
    }

    /// The built-in generic pack.
    pub fn generic() -> RulePack {
        RulePack::from_yaml(GENERIC_RULE_PACK_YAML).expect("built-in generic rule pack must be valid")
    }

    pub fn validate(&self) -> CoreResult<()> {
        let mut seen = std::collections::BTreeSet::new();
        let mut problems = Vec::new();
        if self.rules.is_empty() {
            problems.push("rule pack has no rules".to_string());
        }
        for r in &self.rules {
            if !seen.insert(r.id.clone()) {
                problems.push(format!("duplicate rule id '{}'", r.id));
            }
            if r.severity.parse::<Severity>().is_err() {
                problems.push(format!("rule '{}': invalid severity '{}'", r.id, r.severity));
            }
            if let Some(f) = &r.scope.filter {
                if let Some(p) = &f.name_pattern {
                    if let Err(e) = regex::Regex::new(p) {
                        problems.push(format!("rule '{}': scope.filter.name_pattern invalid regex: {e}", r.id));
                    }
                }
            }
            if let Some(e) = &r.enabled_if {
                if !e.starts_with("$config.") {
                    problems.push(format!("rule '{}': enabled_if must be a '$config.<path>' reference", r.id));
                }
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(CoreError::RulePack(problems.join("; ")))
        }
    }
}

/// Load the rule pack named in `config.requirements.rule_pack`: "generic" or a YAML path
/// (relative to the config directory).
pub fn load_rule_pack(config: &Config, config_dir: Option<&Path>) -> CoreResult<RulePack> {
    let name = config.requirements.rule_pack.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("generic") {
        return Ok(RulePack::generic());
    }
    let path = Config::resolve_path(config_dir, name);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| CoreError::RulePack(format!("cannot read rule pack {}: {e}", path.display())))?;
    RulePack::from_yaml(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_pack_loads_and_has_unique_ids() {
        let p = RulePack::generic();
        assert!(p.rules.len() >= 15, "generic pack should be comprehensive, got {}", p.rules.len());
        let mut ids: Vec<_> = p.rules.iter().map(|r| r.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), p.rules.len());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let y = r#"
name: t
rules:
  - id: A
    category: identity
    description: a
    check: { kind: unique_global_id }
  - id: A
    category: identity
    description: b
    check: { kind: valid_global_id }
"#;
        assert!(RulePack::from_yaml(y).unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn unknown_check_kind_is_rejected() {
        let y = "name: t\nrules:\n  - id: A\n    category: x\n    description: a\n    check: { kind: teleport }\n";
        assert!(RulePack::from_yaml(y).is_err());
    }
}
