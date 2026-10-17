//! `project.yaml` configuration: everything project specific lives here.
//!
//! The struct derives a JSON schema (`schemars`) so the CLI can validate a
//! file and the UI can render a form. Every field has a default so a first
//! run needs zero configuration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// UI / report language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Es,
}

impl Language {
    pub fn code(&self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Es => "es",
        }
    }
}

/// Display unit preference. `Auto` follows the IFC file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum UnitPreference {
    #[default]
    Auto,
    Metric,
    Imperial,
}

/// Top-level `project.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub project: ProjectSection,
    pub requirements: RequirementsSection,
    pub tolerances: TolerancesSection,
    pub qto: QtoSection,
    pub clash: ClashSection,
    pub output: OutputSection,
    /// Per-rule and per-category toggles. Keys are rule ids ("ID-001") or
    /// category names ("geometry"). `false` disables.
    pub checks: ChecksSection,
    /// Naming conventions: IFC type (or ancestor) -> regex the Name must match.
    pub naming: NamingSection,
    /// Classification settings.
    pub classification: ClassificationSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectSection {
    pub name: String,
    pub language: Language,
    pub units: UnitPreference,
    /// Name of the classification system expected ("Uniclass", "OmniClass", "CCI", "DIN276", "none").
    pub classification_system: String,
    /// Accepted schemas. Empty = accept any.
    pub ifc_versions: Vec<String>,
}

impl Default for ProjectSection {
    fn default() -> Self {
        Self {
            name: "Generic Project".into(),
            language: Language::En,
            units: UnitPreference::Auto,
            classification_system: "none".into(),
            ifc_versions: vec!["IFC2X3".into(), "IFC4".into(), "IFC4X3".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RequirementsSection {
    /// Optional IDS file (relative to the config file).
    pub ids_file: Option<String>,
    /// "generic" (built-in) or a path to a custom rule pack YAML.
    pub rule_pack: String,
    /// Property sets / properties that must exist. Each entry generates a rule.
    pub required_psets: Vec<RequiredPset>,
}

impl Default for RequirementsSection {
    fn default() -> Self {
        Self { ids_file: None, rule_pack: "generic".into(), required_psets: vec![] }
    }
}

/// A required property set, optionally with required properties inside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RequiredPset {
    /// Pset name, e.g. "Pset_WallCommon".
    pub pset: String,
    /// Which elements it applies to (IFC type or ancestor, e.g. "IfcWall", "IfcElement"). Empty = all elements.
    pub applies_to: Vec<String>,
    /// Required property names inside the pset. Empty = only the pset must exist.
    pub properties: Vec<String>,
    /// Severity of a missing pset/property. Default: error.
    pub severity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TolerancesSection {
    pub clash_hard_mm: f64,
    pub clash_clearance_mm: f64,
    /// Elements whose bounding box centre is farther than this from the origin are flagged.
    pub far_from_origin_m: f64,
    pub geometry: GeometryTolerances,
}

impl Default for TolerancesSection {
    fn default() -> Self {
        Self {
            clash_hard_mm: 10.0,
            clash_clearance_mm: 50.0,
            far_from_origin_m: 10_000.0,
            geometry: GeometryTolerances::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GeometryTolerances {
    /// Volumes below this are considered zero / degenerate.
    pub min_volume_m3: f64,
    /// Areas below this are considered zero / degenerate.
    pub min_area_m2: f64,
    /// Elements whose bounding box extent exceeds this are flagged as outside project bounds.
    pub max_extent_m: f64,
}

impl Default for GeometryTolerances {
    fn default() -> Self {
        Self { min_volume_m3: 0.000001, min_area_m2: 0.000001, max_extent_m: 5_000.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct QtoSection {
    /// Any combination of "type", "storey", "classification", "material", "predefined_type", "type_name".
    pub group_by: Vec<String>,
    /// Optional unit rates CSV (code, description, unit, rate, currency, match_type, match_storey, match_classification).
    pub unit_rates_file: Option<String>,
    pub currency: String,
    /// Number of hand-check samples shown in the report.
    pub sample_count: usize,
}

impl Default for QtoSection {
    fn default() -> Self {
        Self {
            group_by: vec!["type".into(), "storey".into()],
            unit_rates_file: None,
            currency: "EUR".into(),
            sample_count: 5,
        }
    }
}

/// Element filter used by clash sets and rule scopes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ElementFilter {
    /// IFC type or ancestor, e.g. "IfcPipeSegment".
    pub ifc_type: Option<String>,
    /// Regex on element Name.
    pub name_pattern: Option<String>,
    /// Storey name.
    pub storey: Option<String>,
    /// Required property value: pset.property = value.
    pub property_equals: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ClashSection {
    pub enabled: bool,
    pub set_a_filter: ElementFilter,
    pub set_b_filter: ElementFilter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct OutputSection {
    pub language: Language,
    pub include_3d_snapshot: bool,
    /// BCF version to write: "2.1" (default) or "3.0".
    pub bcf_version: String,
    /// Author recorded in BCF topics.
    pub bcf_author: String,
}

impl Default for OutputSection {
    fn default() -> Self {
        Self {
            language: Language::En,
            include_3d_snapshot: true,
            bcf_version: "2.1".into(),
            bcf_author: "verimodel@local".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ChecksSection {
    /// Rule id -> enabled. Missing = enabled.
    pub rules: BTreeMap<String, bool>,
    /// Category -> enabled. Missing = enabled.
    pub categories: BTreeMap<String, bool>,
    /// Duplicate-name check scope: ["type", "storey"] (default) or ["type"].
    pub unique_name_within: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NamingSection {
    /// IFC type (or ancestor) -> regex. E.g. {"IfcWall": "^W-[0-9]{3}$"}.
    pub patterns: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ClassificationSection {
    /// Require every element in scope to carry a classification.
    pub required: bool,
    /// Which elements must be classified. Empty = all non-spatial elements.
    pub applies_to: Vec<String>,
    /// Allowed codes, inline.
    pub allowed_codes: Vec<String>,
    /// Allowed codes from a file (one code per line, or CSV with `code` column), relative to config.
    pub allowed_codes_file: Option<String>,
    /// Regex that codes must match (e.g. "^[A-Z][a-z]_[0-9]{2}(_[0-9]{2})*$" for Uniclass).
    pub code_pattern: Option<String>,
}

impl ChecksSection {
    /// Whether a rule is enabled given its id and category.
    pub fn is_enabled(&self, rule_id: &str, category: &str) -> bool {
        if let Some(v) = self.rules.get(rule_id) {
            return *v;
        }
        self.categories.get(category).copied().unwrap_or(true)
    }
}

impl Config {
    /// Parse YAML text. Unknown fields are errors so typos never silently disable a rule.
    pub fn from_yaml(text: &str) -> CoreResult<Config> {
        let cfg: Config = serde_yaml::from_str(text).map_err(|e| CoreError::Config(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Read and parse a `project.yaml` file.
    pub fn from_file(path: &Path) -> CoreResult<Config> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| CoreError::Io(format!("cannot read config {}: {e}", path.display())))?;
        Config::from_yaml(&text)
    }

    /// Serialise back to YAML (used to write the effective config into `out/`).
    pub fn to_yaml(&self) -> CoreResult<String> {
        serde_yaml::to_string(self).map_err(|e| CoreError::Config(e.to_string()))
    }

    /// Effective config as JSON (embedded in report.json).
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Semantic validation beyond the schema. Returns a list of plain-language problems.
    pub fn validate(&self) -> CoreResult<()> {
        let mut problems = Vec::new();
        let allowed_groups = ["type", "storey", "classification", "material", "predefined_type", "type_name"];
        for g in &self.qto.group_by {
            if !allowed_groups.contains(&g.as_str()) {
                problems.push(format!(
                    "qto.group_by contains '{g}', expected one of {}",
                    allowed_groups.join(", ")
                ));
            }
        }
        if self.qto.group_by.is_empty() {
            problems.push("qto.group_by must contain at least one key".into());
        }
        if self.tolerances.far_from_origin_m <= 0.0 {
            problems.push("tolerances.far_from_origin_m must be > 0".into());
        }
        if self.tolerances.geometry.min_volume_m3 < 0.0 {
            problems.push("tolerances.geometry.min_volume_m3 must be >= 0".into());
        }
        if !matches!(self.output.bcf_version.as_str(), "2.1" | "3.0") {
            problems.push(format!("output.bcf_version must be '2.1' or '3.0', got '{}'", self.output.bcf_version));
        }
        for (t, p) in &self.naming.patterns {
            if let Err(e) = regex::Regex::new(p) {
                problems.push(format!("naming.patterns['{t}'] is not a valid regex: {e}"));
            }
        }
        if let Some(p) = &self.classification.code_pattern {
            if let Err(e) = regex::Regex::new(p) {
                problems.push(format!("classification.code_pattern is not a valid regex: {e}"));
            }
        }
        for rp in &self.requirements.required_psets {
            if rp.pset.trim().is_empty() {
                problems.push("requirements.required_psets: 'pset' must not be empty".into());
            }
            if let Some(s) = &rp.severity {
                if s.parse::<crate::model::Severity>().is_err() {
                    problems.push(format!("requirements.required_psets['{}'].severity '{s}' is invalid", rp.pset));
                }
            }
        }
        for w in &self.checks.unique_name_within {
            if !matches!(w.as_str(), "type" | "storey") {
                problems.push(format!("checks.unique_name_within contains '{w}', expected 'type' or 'storey'"));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(CoreError::Config(problems.join("; ")))
        }
    }

    /// JSON schema of the configuration, for `validate-config` and the UI form.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Config)).unwrap_or(serde_json::Value::Null)
    }

    /// Resolve a path declared in the config relative to the config file's directory.
    pub fn resolve_path(config_dir: Option<&Path>, declared: &str) -> PathBuf {
        let p = Path::new(declared);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        match config_dir {
            Some(d) => d.join(p),
            None => p.to_path_buf(),
        }
    }

    /// Look up a dotted path ("tolerances.far_from_origin_m") in the config.
    pub fn lookup(&self, dotted: &str) -> Option<serde_json::Value> {
        let mut cur = self.to_json_value();
        for part in dotted.split('.') {
            cur = cur.get(part)?.clone();
        }
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_yaml_gives_defaults() {
        let c = Config::from_yaml("{}").unwrap();
        assert_eq!(c.project.name, "Generic Project");
        assert_eq!(c.qto.group_by, vec!["type", "storey"]);
        assert_eq!(c.tolerances.far_from_origin_m, 10_000.0);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = Config::from_yaml("project:\n  nmae: x\n").unwrap_err();
        assert!(err.to_string().contains("nmae"), "{err}");
    }

    #[test]
    fn invalid_group_by_is_rejected() {
        let err = Config::from_yaml("qto:\n  group_by: [colour]\n").unwrap_err();
        assert!(err.to_string().contains("group_by"));
    }

    #[test]
    fn invalid_regex_is_rejected() {
        let err = Config::from_yaml("naming:\n  patterns:\n    IfcWall: '['\n").unwrap_err();
        assert!(err.to_string().contains("regex"));
    }

    #[test]
    fn checks_toggle_resolution() {
        let c = Config::from_yaml("checks:\n  rules:\n    GEO-001: false\n  categories:\n    naming: false\n").unwrap();
        assert!(!c.checks.is_enabled("GEO-001", "geometry"));
        assert!(c.checks.is_enabled("GEO-002", "geometry"));
        assert!(!c.checks.is_enabled("NAM-001", "naming"));
    }

    #[test]
    fn lookup_dotted_path() {
        let c = Config::default();
        assert_eq!(c.lookup("tolerances.far_from_origin_m"), Some(serde_json::json!(10000.0)));
        assert_eq!(c.lookup("nope.x"), None);
    }

    #[test]
    fn roundtrip_yaml() {
        let c = Config::default();
        let y = c.to_yaml().unwrap();
        let back = Config::from_yaml(&y).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn schema_is_generated() {
        let s = Config::json_schema();
        assert!(s.get("properties").is_some());
    }
}
