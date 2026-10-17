//! Shared data model of VeriModel.
//!
//! Everything in this module is **facts** (what the IFC reader found) or
//! **verdicts** (what the engine decided). The reader fills [`Model`]; the
//! engine produces [`RunReport`]. Nothing here is project specific: element
//! types, property names and storeys are whatever the file contains.
//!
//! All numeric quantities in [`Element`] are expressed in SI base units
//! (metres, square metres, cubic metres, kilograms) regardless of the units
//! declared by the IFC file. The reader is responsible for the conversion and
//! records the original unit system in [`ModelUnits`] so the UI can convert
//! back for display.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A property value as found in an IFC property set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropValue {
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<PropValue>),
    Null,
}

impl PropValue {
    /// True when the value carries no information (null, empty string or empty list).
    pub fn is_empty(&self) -> bool {
        match self {
            PropValue::Null => true,
            PropValue::Text(s) => s.trim().is_empty(),
            PropValue::List(l) => l.is_empty(),
            _ => false,
        }
    }

    /// Human readable rendering used in reports ("expected vs actual").
    pub fn display(&self) -> String {
        match self {
            PropValue::Bool(b) => b.to_string(),
            PropValue::Number(n) => format_number(*n),
            PropValue::Text(s) => s.clone(),
            PropValue::List(l) => l.iter().map(|v| v.display()).collect::<Vec<_>>().join(", "),
            PropValue::Null => "<null>".to_string(),
        }
    }

    /// Name of the JSON-ish data type, used by data-type checks.
    pub fn type_name(&self) -> &'static str {
        match self {
            PropValue::Bool(_) => "boolean",
            PropValue::Number(_) => "number",
            PropValue::Text(_) => "string",
            PropValue::List(_) => "list",
            PropValue::Null => "null",
        }
    }
}

/// Deterministic number formatting (no locale, trailing zeros trimmed).
pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{:.6}", n);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Property sets: pset name -> property name -> value.
pub type PropertySets = BTreeMap<String, BTreeMap<String, PropValue>>;

/// Quantity sets: qset name -> quantity name -> value (SI units).
pub type QuantitySets = BTreeMap<String, BTreeMap<String, f64>>;

/// A classification reference attached to an element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Classification {
    /// Classification system name as declared in the file (e.g. "Uniclass 2015"), if any.
    #[serde(default)]
    pub system: Option<String>,
    /// Classification code / identification (e.g. "Ss_25_10_30").
    #[serde(default)]
    pub code: Option<String>,
    /// Human readable name of the classification item.
    #[serde(default)]
    pub name: Option<String>,
}

/// Reference to a spatial container (site, building, storey, space, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SpatialRef {
    pub global_id: String,
    pub ifc_type: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// Geometry facts for one element, computed by the reader's geometry kernel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Geometry {
    /// Axis-aligned bounding box minimum corner in world coordinates (metres).
    pub bbox_min: [f64; 3],
    /// Axis-aligned bounding box maximum corner in world coordinates (metres).
    pub bbox_max: [f64; 3],
    /// Mesh volume in cubic metres, if the shape is closed.
    #[serde(default)]
    pub volume_m3: Option<f64>,
    /// Mesh surface area in square metres.
    #[serde(default)]
    pub area_m2: Option<f64>,
    /// Number of triangles in the tessellated shape (0 means degenerate).
    #[serde(default)]
    pub triangle_count: u64,
}

impl Geometry {
    /// Distance from the world origin to the bounding box centre (metres).
    pub fn distance_from_origin_m(&self) -> f64 {
        let c = self.centre();
        (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()
    }

    pub fn centre(&self) -> [f64; 3] {
        [
            (self.bbox_min[0] + self.bbox_max[0]) / 2.0,
            (self.bbox_min[1] + self.bbox_max[1]) / 2.0,
            (self.bbox_min[2] + self.bbox_max[2]) / 2.0,
        ]
    }

    pub fn extent(&self) -> [f64; 3] {
        [
            self.bbox_max[0] - self.bbox_min[0],
            self.bbox_max[1] - self.bbox_min[1],
            self.bbox_max[2] - self.bbox_min[2],
        ]
    }
}

/// One IFC product (element or spatial element) as read from the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Element {
    /// IfcRoot.GlobalId exactly as found (may be invalid or duplicated: that is what checks are for).
    pub global_id: String,
    /// STEP instance id (#123) so a coordinator can locate the line in the file.
    #[serde(default)]
    pub step_id: i64,
    /// Entity type, e.g. "IfcWall".
    pub ifc_type: String,
    /// Entity supertypes from most specific to IfcRoot, e.g. ["IfcWall","IfcBuildingElement","IfcElement","IfcProduct",...].
    /// Lets rules target abstract types ("IfcElement") without hardcoding a type list in Rust.
    #[serde(default)]
    pub ancestors: Vec<String>,
    #[serde(default)]
    pub predefined_type: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub object_type: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    /// Name of the IfcTypeObject this element is typed by.
    #[serde(default)]
    pub type_name: Option<String>,
    /// Name of the containing IfcBuildingStorey (resolved through nested spaces).
    #[serde(default)]
    pub storey: Option<String>,
    /// Direct spatial container (via IfcRelContainedInSpatialStructure or IfcRelAggregates for spatial elements).
    #[serde(default)]
    pub container: Option<SpatialRef>,
    #[serde(default)]
    pub psets: PropertySets,
    #[serde(default)]
    pub quantities: QuantitySets,
    #[serde(default)]
    pub classifications: Vec<Classification>,
    #[serde(default)]
    pub materials: Vec<String>,
    /// True when the entity has an IfcProductRepresentation.
    #[serde(default)]
    pub has_representation: bool,
    /// Geometry facts; `None` when the kernel could not build a shape or when there is no representation.
    #[serde(default)]
    pub geometry: Option<Geometry>,
}

impl Element {
    /// Whether the element is of the given type or inherits from it.
    pub fn is_a(&self, ifc_type: &str) -> bool {
        self.ifc_type.eq_ignore_ascii_case(ifc_type)
            || self.ancestors.iter().any(|a| a.eq_ignore_ascii_case(ifc_type))
    }

    /// True for spatial structure elements (site, building, storey, space, facility parts...).
    pub fn is_spatial(&self) -> bool {
        self.is_a("IfcSpatialElement") || self.is_a("IfcSpatialStructureElement")
    }

    /// Look up a property across all psets (`pset` may be `None` to search every set).
    pub fn property(&self, pset: Option<&str>, name: &str) -> Option<&PropValue> {
        match pset {
            Some(p) => self.psets.get(p).and_then(|s| s.get(name)),
            None => self.psets.values().find_map(|s| s.get(name)),
        }
    }

    /// Look up a quantity by name across all quantity sets (first match in sorted set order).
    pub fn quantity(&self, name: &str) -> Option<f64> {
        self.quantities.values().find_map(|s| s.get(name).copied())
    }

    /// Best effort volume in m³: explicit base quantity first, geometry fallback second.
    pub fn volume_m3(&self) -> Option<(f64, QuantitySource)> {
        for key in ["NetVolume", "GrossVolume", "Volume"] {
            if let Some(v) = self.quantity(key) {
                return Some((v, QuantitySource::BaseQuantity));
            }
        }
        self.geometry
            .as_ref()
            .and_then(|g| g.volume_m3)
            .map(|v| (v, QuantitySource::Geometry))
    }

    /// Best effort area in m²: explicit base quantity first, geometry fallback second.
    pub fn area_m2(&self) -> Option<(f64, QuantitySource)> {
        for key in [
            "NetArea",
            "GrossArea",
            "NetSideArea",
            "GrossSideArea",
            "NetFloorArea",
            "GrossFloorArea",
            "OuterSurfaceArea",
            "Area",
        ] {
            if let Some(v) = self.quantity(key) {
                return Some((v, QuantitySource::BaseQuantity));
            }
        }
        self.geometry
            .as_ref()
            .and_then(|g| g.area_m2)
            .map(|v| (v, QuantitySource::Geometry))
    }

    /// Best effort length in m: explicit base quantity first, longest bbox extent second.
    pub fn length_m(&self) -> Option<(f64, QuantitySource)> {
        for key in ["Length", "NetLength", "GrossLength", "Height", "Depth"] {
            if let Some(v) = self.quantity(key) {
                return Some((v, QuantitySource::BaseQuantity));
            }
        }
        self.geometry.as_ref().map(|g| {
            let e = g.extent();
            (e[0].max(e[1]).max(e[2]), QuantitySource::Geometry)
        })
    }

    /// Weight in kg when a base quantity provides it.
    pub fn weight_kg(&self) -> Option<(f64, QuantitySource)> {
        for key in ["NetWeight", "GrossWeight", "Weight"] {
            if let Some(v) = self.quantity(key) {
                return Some((v, QuantitySource::BaseQuantity));
            }
        }
        None
    }
}

/// Where a quantity came from, so every QTO row is auditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuantitySource {
    /// IfcElementQuantity / BaseQuantities present in the file.
    BaseQuantity,
    /// Computed from the tessellated geometry by the reader.
    Geometry,
    /// No value available.
    #[default]
    None,
}

impl QuantitySource {
    pub fn label(&self) -> &'static str {
        match self {
            QuantitySource::BaseQuantity => "base_quantity",
            QuantitySource::Geometry => "geometry",
            QuantitySource::None => "none",
        }
    }
}

/// Unit system detected in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UnitSystem {
    #[default]
    Metric,
    Imperial,
}

/// Units declared by the IFC file (IfcUnitAssignment).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelUnits {
    /// Length unit name as declared, e.g. "MILLIMETRE", "METRE", "INCH", "FOOT".
    #[serde(default)]
    pub length_unit: Option<String>,
    /// Multiply a length in file units by this factor to get metres.
    #[serde(default = "one")]
    pub length_to_m: f64,
    #[serde(default)]
    pub area_unit: Option<String>,
    #[serde(default = "one")]
    pub area_to_m2: f64,
    #[serde(default)]
    pub volume_unit: Option<String>,
    #[serde(default = "one")]
    pub volume_to_m3: f64,
    #[serde(default)]
    pub system: UnitSystem,
}

fn one() -> f64 {
    1.0
}

/// A node of the spatial structure tree (site > building > storey > space, or IFC4.3 facilities).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SpatialNode {
    pub global_id: String,
    pub ifc_type: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Elevation in metres for storeys, if declared.
    #[serde(default)]
    pub elevation_m: Option<f64>,
    #[serde(default)]
    pub children: Vec<SpatialNode>,
}

impl SpatialNode {
    /// Depth-first iterator over all nodes including self.
    pub fn walk(&self) -> Vec<&SpatialNode> {
        let mut out = vec![self];
        for c in &self.children {
            out.extend(c.walk());
        }
        out
    }
}


/// File-level facts from the STEP header and the IFC project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelInfo {
    pub file_name: String,
    pub file_size_bytes: u64,
    pub sha256: String,
    /// Schema identifier as declared in FILE_SCHEMA, normalised: "IFC2X3", "IFC4", "IFC4X3".
    pub schema: String,
    /// Model View Definition from FILE_DESCRIPTION, if any.
    #[serde(default)]
    pub mvd: Option<String>,
    /// Originating application from the header.
    #[serde(default)]
    pub authoring_tool: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    #[serde(default)]
    pub units: ModelUnits,
    pub element_count: usize,
    /// Spatial tree roots (usually one IfcSite or IfcProject children).
    #[serde(default)]
    pub spatial_tree: Vec<SpatialNode>,
    /// Count of products by entity type, sorted by type name.
    #[serde(default)]
    pub type_counts: BTreeMap<String, usize>,
    /// Storey names in elevation order.
    #[serde(default)]
    pub storeys: Vec<String>,
    /// Disciplines inferred from entity types (data-driven, see data/disciplines.yaml).
    #[serde(default)]
    pub disciplines: Vec<String>,
    /// Non-fatal reader warnings (e.g. geometry failures) in plain language.
    #[serde(default)]
    pub reader_warnings: Vec<String>,
    /// Whether the reader could run the geometry kernel.
    #[serde(default)]
    pub geometry_available: bool,
}

/// The whole input to the engine: facts about one IFC file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Model {
    pub info: ModelInfo,
    pub elements: Vec<Element>,
    /// Raw IDS audit results from the reader (when an IDS file was given).
    #[serde(default)]
    pub ids_results: Option<IdsResults>,
}

impl Model {
    /// Elements sorted by GlobalId then step id, so every consumer iterates deterministically.
    pub fn sort_elements(&mut self) {
        self.elements
            .sort_by(|a, b| a.global_id.cmp(&b.global_id).then(a.step_id.cmp(&b.step_id)));
    }

    pub fn element_by_id(&self, global_id: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.global_id == global_id)
    }

    /// Every GlobalId of a spatial structure node, for containment checks.
    pub fn spatial_ids(&self) -> std::collections::BTreeSet<String> {
        let mut set = std::collections::BTreeSet::new();
        for root in &self.info.spatial_tree {
            for n in root.walk() {
                set.insert(n.global_id.clone());
            }
        }
        set
    }
}

// ---------------------------------------------------------------------------
// IDS results as produced by the reader (facts, no verdict yet).
// ---------------------------------------------------------------------------

/// IDS audit results for one IDS file, as reported by the IDS engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IdsResults {
    pub ids_file: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub specifications: Vec<IdsSpecificationResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IdsSpecificationResult {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    /// Plain text description of the applicability facets ("All IfcWall entities ...").
    #[serde(default)]
    pub applicability: Vec<String>,
    /// "required" | "optional" | "prohibited" (IDS 1.0 cardinality).
    #[serde(default)]
    pub cardinality: Option<String>,
    /// Number of applicable entities found.
    #[serde(default)]
    pub applicable_count: usize,
    /// Whether the whole specification passed.
    pub passed: bool,
    #[serde(default)]
    pub requirements: Vec<IdsRequirementResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IdsRequirementResult {
    /// Plain-language requirement text, quoted verbatim in reports.
    pub description: String,
    /// Facet type: "entity" | "attribute" | "property" | "classification" | "material" | "partOf".
    #[serde(default)]
    pub facet: String,
    pub passed: bool,
    #[serde(default)]
    pub failed_entities: Vec<IdsFailedEntity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IdsFailedEntity {
    pub global_id: String,
    #[serde(default)]
    pub ifc_type: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Reason given by the IDS engine ("property FireRating is missing").
    #[serde(default)]
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Verdicts
// ---------------------------------------------------------------------------

/// Severity of a finding. Ordering: Error > Warning > Info.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
            Severity::Info => "INFO",
        }
    }
}

impl std::str::FromStr for Severity {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Ok(Severity::Error),
            "warning" | "warn" => Ok(Severity::Warning),
            "info" => Ok(Severity::Info),
            other => Err(format!("unknown severity '{other}' (expected error|warning|info)")),
        }
    }
}

/// Outcome of one rule over the whole model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RuleStatus {
    Pass,
    Fail,
    /// The rule could not run (e.g. geometry kernel unavailable). Never counted as PASS.
    Skipped,
    /// The rule applied to zero elements.
    NotApplicable,
}

/// Where a rule came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleSource {
    /// Built-in or custom rule pack (YAML).
    RulePack { pack: String },
    /// Generated from `requirements.required_psets` in project.yaml.
    ProjectConfig,
    /// buildingSMART IDS specification.
    Ids { specification: String, requirement: String },
}

/// One failure: one element (or the model as a whole) failing one rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub rule_id: String,
    pub severity: Severity,
    /// `None` for model-level findings (e.g. missing spatial hierarchy).
    #[serde(default)]
    pub global_id: Option<String>,
    #[serde(default)]
    pub step_id: Option<i64>,
    #[serde(default)]
    pub ifc_type: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub storey: Option<String>,
    /// What went wrong, in plain language.
    pub message: String,
    #[serde(default)]
    pub expected: Option<String>,
    #[serde(default)]
    pub actual: Option<String>,
    #[serde(default)]
    pub fix_hint: Option<String>,
    pub source: RuleSource,
}

/// Per-rule summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleResult {
    pub rule_id: String,
    pub category: String,
    pub description: String,
    pub severity: Severity,
    pub status: RuleStatus,
    /// Elements the rule was applied to.
    pub elements_checked: usize,
    /// Number of findings produced by the rule.
    pub failures: usize,
    /// Plain-language reason when `status == Skipped`.
    #[serde(default)]
    pub skipped_reason: Option<String>,
    pub source: RuleSource,
}

/// Overall verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    PassWithWarnings,
    Fail,
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::PassWithWarnings => "PASS WITH WARNINGS",
            Verdict::Fail => "FAIL",
        }
    }
}

/// Headline numbers for the review screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Summary {
    /// 0..=100. See [`crate::score`] for the documented formula.
    pub score: u8,
    pub verdict: Option<Verdict>,
    /// Distinct elements evaluated by at least one rule.
    pub elements_checked: usize,
    /// Sum over rules of elements evaluated (the denominator of the score).
    pub element_checks: usize,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub rules_total: usize,
    pub rules_passed: usize,
    pub rules_failed: usize,
    pub rules_skipped: usize,
    pub rules_not_applicable: usize,
    /// Plain-language statement of the verdict rule, shown in the UI.
    pub verdict_rule: String,
}

/// One row of the quantity takeoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct QtoRow {
    /// Group key values in `group_by` order (e.g. ["IfcWall", "Level 1", "Ss_25"]).
    pub group: Vec<String>,
    pub count: usize,
    pub volume_m3: f64,
    pub area_m2: f64,
    pub length_m: f64,
    pub weight_kg: f64,
    /// How many elements of the group had each quantity from base quantities vs geometry vs none.
    pub volume_sources: BTreeMap<String, usize>,
    pub area_sources: BTreeMap<String, usize>,
    pub length_sources: BTreeMap<String, usize>,
    /// Cost line when a unit rate matched this group.
    #[serde(default)]
    pub cost: Option<CostLine>,
}

/// Cost estimate for a QTO row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CostLine {
    pub rate_code: String,
    pub rate_description: String,
    /// Unit the rate applies to: "m3" | "m2" | "m" | "kg" | "nr".
    pub unit: String,
    pub rate: f64,
    pub quantity: f64,
    pub amount: f64,
    pub currency: String,
    /// File the rate came from.
    pub source: String,
}

/// A hand-check sample so a reader can verify the QTO by hand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct QtoSample {
    pub global_id: String,
    pub ifc_type: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub storey: Option<String>,
    #[serde(default)]
    pub volume_m3: Option<f64>,
    pub volume_source: QuantitySource,
    #[serde(default)]
    pub area_m2: Option<f64>,
    pub area_source: QuantitySource,
    #[serde(default)]
    pub length_m: Option<f64>,
    pub length_source: QuantitySource,
    /// Quantity set names found on the element.
    pub quantity_sets: Vec<String>,
}

/// Quantity takeoff section of the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct QtoReport {
    /// Grouping keys used, e.g. ["type", "storey"].
    pub group_by: Vec<String>,
    pub rows: Vec<QtoRow>,
    pub total_count: usize,
    pub total_volume_m3: f64,
    pub total_area_m2: f64,
    pub total_length_m: f64,
    pub total_weight_kg: f64,
    #[serde(default)]
    pub total_cost: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
    /// Elements with no usable quantity at all.
    pub elements_without_quantities: usize,
    /// Plain-language explanation of how totals were computed.
    pub method: String,
    pub samples: Vec<QtoSample>,
}

/// Clash detection section (phase 2). Present so the report shape is stable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ClashReport {
    pub enabled: bool,
    pub implemented: bool,
    pub message: String,
    #[serde(default)]
    pub clashes: Vec<Clash>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Clash {
    pub global_id_a: String,
    pub global_id_b: String,
    /// "hard" | "clearance" | "duplicate"
    pub clash_type: String,
    pub distance_m: f64,
    pub location: [f64; 3],
}

/// IDS section of the report: what the IDS engine said, quoted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IdsReport {
    pub ids_file: String,
    #[serde(default)]
    pub title: Option<String>,
    pub specifications_total: usize,
    pub specifications_passed: usize,
    pub specifications: Vec<IdsSpecificationResult>,
}

/// Tool identity recorded in every output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
}

impl Default for ToolInfo {
    fn default() -> Self {
        Self { name: crate::TOOL_NAME.to_string(), version: crate::VERSION.to_string() }
    }
}

/// The complete, deterministic result of one run. Serialised as `report.json`.
///
/// Must not contain timestamps or durations: those live in `run.json` so that
/// `report.json` is byte-for-byte reproducible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RunReport {
    pub tool: ToolInfo,
    pub model: ModelInfo,
    /// The effective configuration used for this run (serialised project.yaml).
    pub config: serde_json::Value,
    pub summary: Summary,
    pub rules: Vec<RuleResult>,
    pub findings: Vec<Finding>,
    pub qto: QtoReport,
    pub ids: Option<IdsReport>,
    pub clash: ClashReport,
}

impl RunReport {
    /// Sort rules and findings so output is deterministic regardless of evaluation order.
    pub fn normalise(&mut self) {
        self.rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
        self.findings.sort_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then_with(|| a.rule_id.cmp(&b.rule_id))
                .then_with(|| a.global_id.cmp(&b.global_id))
                .then_with(|| a.message.cmp(&b.message))
        });
    }
}

/// Run ledger: reproducibility and audit data. Serialised as `run.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RunLedger {
    pub tool: ToolInfo,
    /// ISO-8601 UTC timestamp.
    pub timestamp: String,
    pub duration_ms: u64,
    pub ifc_file: String,
    pub ifc_sha256: String,
    #[serde(default)]
    pub ids_file: Option<String>,
    #[serde(default)]
    pub ids_sha256: Option<String>,
    #[serde(default)]
    pub config_file: Option<String>,
    pub config_sha256: String,
    #[serde(default)]
    pub rates_file: Option<String>,
    #[serde(default)]
    pub rates_sha256: Option<String>,
    pub schema: String,
    pub element_count: usize,
    pub score: u8,
    pub verdict: Option<Verdict>,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub rules_passed: usize,
    pub rules_failed: usize,
    pub rules_skipped: usize,
    /// SHA256 of report.json, so anyone can prove which report belongs to this run.
    pub report_sha256: String,
    pub output_dir: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_number_is_stable() {
        assert_eq!(format_number(2.0), "2");
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(0.1234567), "0.123457");
        assert_eq!(format_number(-3.0), "-3");
    }

    #[test]
    fn element_is_a_uses_ancestors() {
        let e = Element {
            ifc_type: "IfcWallStandardCase".into(),
            ancestors: vec!["IfcWall".into(), "IfcBuildingElement".into(), "IfcElement".into()],
            ..Default::default()
        };
        assert!(e.is_a("IfcElement"));
        assert!(e.is_a("ifcwall"));
        assert!(!e.is_a("IfcSlab"));
    }

    #[test]
    fn volume_prefers_base_quantity_over_geometry() {
        let mut e = Element::default();
        e.geometry = Some(Geometry { volume_m3: Some(5.0), ..Default::default() });
        assert_eq!(e.volume_m3(), Some((5.0, QuantitySource::Geometry)));
        e.quantities.entry("Qto_WallBaseQuantities".into()).or_default().insert("NetVolume".into(), 2.0);
        assert_eq!(e.volume_m3(), Some((2.0, QuantitySource::BaseQuantity)));
    }

    #[test]
    fn severity_orders_error_first() {
        assert!(Severity::Error < Severity::Warning);
        assert!(Severity::Warning < Severity::Info);
    }
}
