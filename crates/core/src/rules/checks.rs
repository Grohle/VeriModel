//! Check kinds: the engine's vocabulary. Rules choose a kind and parameters;
//! the engine applies the kind to the elements in scope.
//!
//! Every check returns findings (facts about what failed). Severity, rule id
//! and fix hints are attached by the engine, so a check never needs to know
//! which rule invoked it.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{CoreError, CoreResult};
use crate::model::{Element, Model, PropValue};

/// A parameter that is either a literal or a `$config.<dotted.path>` reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Param {
    Number(f64),
    Bool(bool),
    Text(String),
    List(Vec<String>),
}

impl Param {
    /// Resolve `$config.` references against the config. Literals pass through.
    pub fn resolve(&self, config: &Config) -> CoreResult<serde_json::Value> {
        match self {
            Param::Text(s) if s.starts_with("$config.") => {
                let path = &s["$config.".len()..];
                config.lookup(path).ok_or_else(|| {
                    CoreError::RulePack(format!("parameter '{s}' refers to an unknown config path"))
                })
            }
            Param::Number(n) => Ok(serde_json::json!(n)),
            Param::Bool(b) => Ok(serde_json::json!(b)),
            Param::Text(s) => Ok(serde_json::json!(s)),
            Param::List(l) => Ok(serde_json::json!(l)),
        }
    }

    pub fn as_f64(&self, config: &Config) -> CoreResult<f64> {
        let v = self.resolve(config)?;
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(|| CoreError::RulePack(format!("parameter {:?} is not a number", self)))
    }

    pub fn as_bool(&self, config: &Config) -> CoreResult<bool> {
        let v = self.resolve(config)?;
        v.as_bool()
            .ok_or_else(|| CoreError::RulePack(format!("parameter {:?} is not a boolean", self)))
    }

    pub fn as_string_list(&self, config: &Config) -> CoreResult<Vec<String>> {
        let v = self.resolve(config)?;
        match v {
            serde_json::Value::Array(a) => Ok(a
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()),
            serde_json::Value::String(s) => Ok(vec![s]),
            _ => Err(CoreError::RulePack(format!(
                "parameter {:?} is not a list",
                self
            ))),
        }
    }
}

/// Which quantity a quantity check looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuantityKind {
    Volume,
    Area,
    Length,
}

/// The engine's check vocabulary. Serialised with a `kind` tag in YAML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CheckKind {
    /// GlobalId must be unique in the model.
    UniqueGlobalId,
    /// GlobalId must be a 22 character IFC base64 string.
    ValidGlobalId,
    /// Name must be unique within the given keys (default from `config.checks.unique_name_within`).
    UniqueName {
        #[serde(default)]
        within: Option<Vec<String>>,
    },
    /// Element must be contained in (or aggregated into) a spatial structure element.
    ContainedInSpatialStructure,
    /// The element's container GlobalId must exist in the spatial tree.
    ContainerExists,
    /// Model-level: every group in `require` must have at least one node in the spatial tree.
    /// Each group is a list of alternatives, e.g. [["IfcSite"], ["IfcBuilding", "IfcBridge", "IfcRoad"]].
    SpatialHierarchy { require: Vec<Vec<String>> },
    /// Bounding-box centre must be within `max_m` of the origin.
    DistanceFromOrigin { max_m: Param },
    /// Property set must exist.
    PsetExists { pset: String },
    /// Property must exist (in `pset`, or any pset when omitted) and, if `require_value`, be non-empty.
    PropertyExists {
        #[serde(default)]
        pset: Option<String>,
        property: String,
        #[serde(default = "default_true")]
        require_value: bool,
        /// Expected data type: "string" | "number" | "boolean".
        #[serde(default)]
        data_type: Option<String>,
    },
    /// Property value must match a regex and/or be in an allowed list.
    PropertyMatches {
        #[serde(default)]
        pset: Option<String>,
        property: String,
        #[serde(default)]
        pattern: Option<Param>,
        #[serde(default)]
        allowed_values: Option<Param>,
    },
    /// No property in any pset may be empty / null.
    NoEmptyProperties,
    /// Element must have a product representation.
    HasGeometry,
    /// Element with a representation must produce a non-degenerate shape (triangles, volume or area above tolerance).
    NonDegenerateGeometry {
        #[serde(default)]
        min_volume_m3: Option<Param>,
        #[serde(default)]
        min_area_m2: Option<Param>,
    },
    /// Bounding-box extent must not exceed `max_extent_m`.
    BboxWithinBounds { max_extent_m: Param },
    /// Element must carry at least one classification reference (optionally from a given system).
    ClassificationExists {
        #[serde(default)]
        system: Option<Param>,
    },
    /// Classification code must be in `config.classification.allowed_codes` / file / match `code_pattern`.
    ClassificationCodeAllowed,
    /// Name must match a regex (explicit `pattern`, or per-type patterns from `config.naming.patterns`).
    NameMatches {
        #[serde(default)]
        pattern: Option<Param>,
    },
    /// A quantity (from base quantities or geometry) must be greater than `min`.
    QuantityPositive {
        quantity: QuantityKind,
        #[serde(default)]
        min: Option<Param>,
    },
    /// Model-level: every listed type must have at least one element.
    TypesPresent { ifc_types: Param },
    /// Model-level: the file schema must be one of `config.project.ifc_versions`.
    SchemaAllowed,
}

fn default_true() -> bool {
    true
}

/// A raw failure produced by a check, before the engine attaches rule metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckFailure {
    /// `None` for model-level failures.
    pub element: Option<usize>,
    pub message: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

impl CheckFailure {
    fn element(
        idx: usize,
        message: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            element: Some(idx),
            message: message.into(),
            expected: Some(expected.into()),
            actual: Some(actual.into()),
        }
    }
    fn model(
        message: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            element: None,
            message: message.into(),
            expected: Some(expected.into()),
            actual: Some(actual.into()),
        }
    }
}

/// Result of running one check over its scope.
#[derive(Debug, Clone, PartialEq)]
pub enum CheckOutcome {
    /// Ran; `failures` may be empty.
    Ran { failures: Vec<CheckFailure> },
    /// Could not run: plain-language reason.
    Skipped { reason: String },
}

/// Context given to every check.
pub struct CheckContext<'a> {
    pub model: &'a Model,
    pub config: &'a Config,
    /// Indices into `model.elements` that are in scope for this rule.
    pub scope: &'a [usize],
    /// Allowed classification codes resolved from config (inline + file), lazily computed by the engine.
    pub allowed_codes: &'a BTreeSet<String>,
}

const GLOBAL_ID_ALPHABET: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";

/// Whether a string is a syntactically valid IFC GlobalId (22 chars, IFC base64 alphabet, first char 0..3).
pub fn is_valid_global_id(id: &str) -> bool {
    id.len() == 22
        && id.chars().all(|c| GLOBAL_ID_ALPHABET.contains(c))
        && id
            .chars()
            .next()
            .map(|c| ('0'..='3').contains(&c))
            .unwrap_or(false)
}

fn name_of(e: &Element) -> String {
    e.name.clone().unwrap_or_default()
}

fn describe_elem(e: &Element) -> String {
    match &e.name {
        Some(n) if !n.is_empty() => format!("{} '{}'", e.ifc_type, n),
        _ => format!("{} #{}", e.ifc_type, e.step_id),
    }
}

impl CheckKind {
    /// Human readable name of the kind, for documentation.
    pub fn kind_name(&self) -> &'static str {
        match self {
            CheckKind::UniqueGlobalId => "unique_global_id",
            CheckKind::ValidGlobalId => "valid_global_id",
            CheckKind::UniqueName { .. } => "unique_name",
            CheckKind::ContainedInSpatialStructure => "contained_in_spatial_structure",
            CheckKind::ContainerExists => "container_exists",
            CheckKind::SpatialHierarchy { .. } => "spatial_hierarchy",
            CheckKind::DistanceFromOrigin { .. } => "distance_from_origin",
            CheckKind::PsetExists { .. } => "pset_exists",
            CheckKind::PropertyExists { .. } => "property_exists",
            CheckKind::PropertyMatches { .. } => "property_matches",
            CheckKind::NoEmptyProperties => "no_empty_properties",
            CheckKind::HasGeometry => "has_geometry",
            CheckKind::NonDegenerateGeometry { .. } => "non_degenerate_geometry",
            CheckKind::BboxWithinBounds { .. } => "bbox_within_bounds",
            CheckKind::ClassificationExists { .. } => "classification_exists",
            CheckKind::ClassificationCodeAllowed => "classification_code_allowed",
            CheckKind::NameMatches { .. } => "name_matches",
            CheckKind::QuantityPositive { .. } => "quantity_positive",
            CheckKind::TypesPresent { .. } => "types_present",
            CheckKind::SchemaAllowed => "schema_allowed",
        }
    }

    /// True for checks that evaluate the model as a whole rather than each element.
    pub fn is_model_level(&self) -> bool {
        matches!(
            self,
            CheckKind::SpatialHierarchy { .. }
                | CheckKind::TypesPresent { .. }
                | CheckKind::SchemaAllowed
        )
    }

    /// Run the check.
    pub fn run(&self, ctx: &CheckContext<'_>) -> CoreResult<CheckOutcome> {
        let els = &ctx.model.elements;
        let cfg = ctx.config;
        let mut failures = Vec::new();
        match self {
            CheckKind::UniqueGlobalId => {
                let mut by_id: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
                for &i in ctx.scope {
                    by_id.entry(els[i].global_id.as_str()).or_default().push(i);
                }
                for (id, idxs) in by_id {
                    if idxs.len() > 1 {
                        let others: Vec<String> = idxs
                            .iter()
                            .map(|i| format!("#{}", els[*i].step_id))
                            .collect();
                        for &i in &idxs {
                            failures.push(CheckFailure::element(
                                i,
                                format!(
                                    "GlobalId '{id}' is used by {} elements ({})",
                                    idxs.len(),
                                    others.join(", ")
                                ),
                                "unique GlobalId",
                                format!("{} occurrences", idxs.len()),
                            ));
                        }
                    }
                }
            }
            CheckKind::ValidGlobalId => {
                for &i in ctx.scope {
                    let id = &els[i].global_id;
                    if !is_valid_global_id(id) {
                        failures.push(CheckFailure::element(
                            i,
                            format!("GlobalId '{id}' is not a valid 22-character IFC identifier"),
                            "22 characters from the IFC base64 alphabet",
                            if id.is_empty() {
                                "<empty>".to_string()
                            } else {
                                format!("'{id}' ({} chars)", id.len())
                            },
                        ));
                    }
                }
            }
            CheckKind::UniqueName { within } => {
                let keys: Vec<String> = within
                    .clone()
                    .or_else(|| {
                        if cfg.checks.unique_name_within.is_empty() {
                            None
                        } else {
                            Some(cfg.checks.unique_name_within.clone())
                        }
                    })
                    .unwrap_or_else(|| vec!["type".into(), "storey".into()]);
                let mut groups: BTreeMap<Vec<String>, Vec<usize>> = BTreeMap::new();
                for &i in ctx.scope {
                    let e = &els[i];
                    let name = name_of(e);
                    if name.trim().is_empty() {
                        continue;
                    }
                    let mut key = vec![name];
                    for k in &keys {
                        match k.as_str() {
                            "type" => key.push(e.ifc_type.clone()),
                            "storey" => key.push(e.storey.clone().unwrap_or_default()),
                            _ => {}
                        }
                    }
                    groups.entry(key).or_default().push(i);
                }
                for (key, idxs) in groups {
                    if idxs.len() > 1 {
                        for &i in &idxs {
                            failures.push(CheckFailure::element(
                                i,
                                format!(
                                    "Name '{}' is shared by {} elements of the same {}",
                                    key[0],
                                    idxs.len(),
                                    keys.join(" and ")
                                ),
                                "unique name",
                                format!("{} elements named '{}'", idxs.len(), key[0]),
                            ));
                        }
                    }
                }
            }
            CheckKind::ContainedInSpatialStructure => {
                for &i in ctx.scope {
                    if els[i].container.is_none() {
                        failures.push(CheckFailure::element(
                            i,
                            format!("{} is not contained in any storey, space, site or other spatial element", describe_elem(&els[i])),
                            "IfcRelContainedInSpatialStructure to a spatial element",
                            "no spatial container",
                        ));
                    }
                }
            }
            CheckKind::ContainerExists => {
                let spatial = ctx.model.spatial_ids();
                for &i in ctx.scope {
                    if let Some(c) = &els[i].container {
                        if !spatial.contains(&c.global_id) {
                            failures.push(CheckFailure::element(
                                i,
                                format!(
                                    "{} is assigned to container {} '{}' which is not part of the spatial hierarchy",
                                    describe_elem(&els[i]),
                                    c.ifc_type,
                                    c.name.clone().unwrap_or_default()
                                ),
                                "container present in the site/building/storey tree",
                                format!("container {} ({})", c.global_id, c.ifc_type),
                            ));
                        }
                    }
                }
            }
            CheckKind::SpatialHierarchy { require } => {
                let present: BTreeSet<String> = ctx
                    .model
                    .info
                    .spatial_tree
                    .iter()
                    .flat_map(|r| r.walk())
                    .map(|n| n.ifc_type.to_ascii_lowercase())
                    .collect();
                for group in require {
                    let ok = group
                        .iter()
                        .any(|t| present.contains(&t.to_ascii_lowercase()));
                    if !ok {
                        failures.push(CheckFailure::model(
                            format!(
                                "Spatial hierarchy is missing a {} element",
                                group.join(" / ")
                            ),
                            format!("at least one of {}", group.join(", ")),
                            "none found",
                        ));
                    }
                }
            }
            CheckKind::DistanceFromOrigin { max_m } => {
                if !ctx.model.info.geometry_available {
                    return Ok(CheckOutcome::Skipped {
                        reason: "geometry kernel unavailable".into(),
                    });
                }
                let max = max_m.as_f64(cfg)?;
                for &i in ctx.scope {
                    if let Some(g) = &els[i].geometry {
                        let d = g.distance_from_origin_m();
                        if d > max {
                            failures.push(CheckFailure::element(
                                i,
                                format!(
                                    "{} is {:.1} m from the project origin",
                                    describe_elem(&els[i]),
                                    d
                                ),
                                format!("<= {} m", crate::model::format_number(max)),
                                format!("{:.1} m", d),
                            ));
                        }
                    }
                }
            }
            CheckKind::PsetExists { pset } => {
                for &i in ctx.scope {
                    if !els[i].psets.contains_key(pset) {
                        failures.push(CheckFailure::element(
                            i,
                            format!("{} has no property set '{pset}'", describe_elem(&els[i])),
                            format!("property set {pset}"),
                            format!("sets present: {}", list_or_none(els[i].psets.keys())),
                        ));
                    }
                }
            }
            CheckKind::PropertyExists {
                pset,
                property,
                require_value,
                data_type,
            } => {
                for &i in ctx.scope {
                    let e = &els[i];
                    let label = match pset {
                        Some(p) => format!("{p}.{property}"),
                        None => property.clone(),
                    };
                    match e.property(pset.as_deref(), property) {
                        None => failures.push(CheckFailure::element(
                            i,
                            format!("{} is missing property {label}", describe_elem(e)),
                            format!("property {label} present"),
                            "missing",
                        )),
                        Some(v) if *require_value && v.is_empty() => {
                            failures.push(CheckFailure::element(
                                i,
                                format!("{} has property {label} but no value", describe_elem(e)),
                                format!("non-empty value for {label}"),
                                v.display(),
                            ))
                        }
                        Some(v) => {
                            if let Some(dt) = data_type {
                                if !type_matches(v, dt) {
                                    failures.push(CheckFailure::element(
                                        i,
                                        format!(
                                            "{} property {label} has the wrong data type",
                                            describe_elem(e)
                                        ),
                                        dt.clone(),
                                        format!("{} ({})", v.type_name(), v.display()),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            CheckKind::PropertyMatches {
                pset,
                property,
                pattern,
                allowed_values,
            } => {
                let re = match pattern {
                    Some(p) => {
                        let s = p.resolve(cfg)?.as_str().unwrap_or_default().to_string();
                        Some(Regex::new(&s).map_err(|e| {
                            CoreError::RulePack(format!("invalid pattern '{s}': {e}"))
                        })?)
                    }
                    None => None,
                };
                let allowed = match allowed_values {
                    Some(a) => Some(a.as_string_list(cfg)?),
                    None => None,
                };
                for &i in ctx.scope {
                    let e = &els[i];
                    let Some(v) = e.property(pset.as_deref(), property) else {
                        continue;
                    };
                    let s = v.display();
                    if let Some(re) = &re {
                        if !re.is_match(&s) {
                            failures.push(CheckFailure::element(
                                i,
                                format!("{} property {property} = '{s}' does not match the required pattern", describe_elem(e)),
                                format!("matches /{}/", re.as_str()),
                                s.clone(),
                            ));
                            continue;
                        }
                    }
                    if let Some(a) = &allowed {
                        if !a.iter().any(|x| x == &s) {
                            failures.push(CheckFailure::element(
                                i,
                                format!(
                                    "{} property {property} = '{s}' is not an allowed value",
                                    describe_elem(e)
                                ),
                                format!("one of {}", a.join(", ")),
                                s.clone(),
                            ));
                        }
                    }
                }
            }
            CheckKind::NoEmptyProperties => {
                for &i in ctx.scope {
                    let e = &els[i];
                    let mut empties = Vec::new();
                    for (ps, props) in &e.psets {
                        for (k, v) in props {
                            if v.is_empty() {
                                empties.push(format!("{ps}.{k}"));
                            }
                        }
                    }
                    if !empties.is_empty() {
                        failures.push(CheckFailure::element(
                            i,
                            format!(
                                "{} has {} empty propert{}: {}",
                                describe_elem(e),
                                empties.len(),
                                if empties.len() == 1 { "y" } else { "ies" },
                                empties.join(", ")
                            ),
                            "every property carries a value",
                            format!("{} empty", empties.len()),
                        ));
                    }
                }
            }
            CheckKind::HasGeometry => {
                for &i in ctx.scope {
                    if !els[i].has_representation {
                        failures.push(CheckFailure::element(
                            i,
                            format!("{} has no geometric representation", describe_elem(&els[i])),
                            "IfcProductDefinitionShape with at least one representation",
                            "no representation",
                        ));
                    }
                }
            }
            CheckKind::NonDegenerateGeometry {
                min_volume_m3,
                min_area_m2,
            } => {
                if !ctx.model.info.geometry_available {
                    return Ok(CheckOutcome::Skipped {
                        reason: "geometry kernel unavailable".into(),
                    });
                }
                let min_v = match min_volume_m3 {
                    Some(p) => p.as_f64(cfg)?,
                    None => cfg.tolerances.geometry.min_volume_m3,
                };
                let min_a = match min_area_m2 {
                    Some(p) => p.as_f64(cfg)?,
                    None => cfg.tolerances.geometry.min_area_m2,
                };
                for &i in ctx.scope {
                    let e = &els[i];
                    if !e.has_representation {
                        continue;
                    }
                    match &e.geometry {
                        None => failures.push(CheckFailure::element(
                            i,
                            format!("{} has a representation but the geometry kernel could not build a shape", describe_elem(e)),
                            "a valid shape",
                            "shape generation failed",
                        )),
                        Some(g) => {
                            let vol = g.volume_m3.unwrap_or(0.0);
                            let area = g.area_m2.unwrap_or(0.0);
                            if g.triangle_count == 0 || (vol.abs() <= min_v && area <= min_a) {
                                failures.push(CheckFailure::element(
                                    i,
                                    format!("{} has degenerate geometry (volume {:.6} m³, area {:.6} m², {} triangles)", describe_elem(e), vol, area, g.triangle_count),
                                    format!("volume > {} m³ or area > {} m²", crate::model::format_number(min_v), crate::model::format_number(min_a)),
                                    format!("volume {:.6} m³, area {:.6} m²", vol, area),
                                ));
                            }
                        }
                    }
                }
            }
            CheckKind::BboxWithinBounds { max_extent_m } => {
                if !ctx.model.info.geometry_available {
                    return Ok(CheckOutcome::Skipped {
                        reason: "geometry kernel unavailable".into(),
                    });
                }
                let max = max_extent_m.as_f64(cfg)?;
                for &i in ctx.scope {
                    if let Some(g) = &els[i].geometry {
                        let ext = g.extent();
                        let largest = ext[0].max(ext[1]).max(ext[2]);
                        if largest > max {
                            failures.push(CheckFailure::element(
                                i,
                                format!(
                                    "{} spans {:.1} m, larger than the configured project bounds",
                                    describe_elem(&els[i]),
                                    largest
                                ),
                                format!("extent <= {} m", crate::model::format_number(max)),
                                format!("{:.1} m", largest),
                            ));
                        }
                    }
                }
            }
            CheckKind::ClassificationExists { system } => {
                let sys = match system {
                    Some(p) => p
                        .resolve(cfg)?
                        .as_str()
                        .map(|s| s.to_string())
                        .filter(|s| !s.is_empty() && s != "none"),
                    None => None,
                };
                // `classification.applies_to` narrows the scope without a custom rule pack.
                let applies = &cfg.classification.applies_to;
                for &i in ctx.scope {
                    let e = &els[i];
                    if !applies.is_empty() && !applies.iter().any(|t| e.is_a(t)) {
                        continue;
                    }
                    let found = e.classifications.iter().any(|c| {
                        c.code.as_ref().map(|s| !s.is_empty()).unwrap_or(false)
                            && match &sys {
                                Some(s) => c
                                    .system
                                    .as_ref()
                                    .map(|x| x.to_lowercase().contains(&s.to_lowercase()))
                                    .unwrap_or(false),
                                None => true,
                            }
                    });
                    if !found {
                        failures.push(CheckFailure::element(
                            i,
                            match &sys {
                                Some(s) => {
                                    format!("{} has no {s} classification", describe_elem(e))
                                }
                                None => {
                                    format!("{} has no classification reference", describe_elem(e))
                                }
                            },
                            match &sys {
                                Some(s) => format!("classification from system '{s}'"),
                                None => "at least one IfcClassificationReference".to_string(),
                            },
                            if e.classifications.is_empty() {
                                "none".to_string()
                            } else {
                                e.classifications
                                    .iter()
                                    .map(|c| {
                                        format!(
                                            "{}:{}",
                                            c.system.clone().unwrap_or_default(),
                                            c.code.clone().unwrap_or_default()
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            },
                        ));
                    }
                }
            }
            CheckKind::ClassificationCodeAllowed => {
                let pattern = match &cfg.classification.code_pattern {
                    Some(p) => Some(Regex::new(p).map_err(|e| {
                        CoreError::Config(format!("classification.code_pattern: {e}"))
                    })?),
                    None => None,
                };
                if ctx.allowed_codes.is_empty() && pattern.is_none() {
                    return Ok(CheckOutcome::Skipped {
                        reason: "no allowed classification codes or code pattern configured (classification.allowed_codes / allowed_codes_file / code_pattern)".into(),
                    });
                }
                for &i in ctx.scope {
                    let e = &els[i];
                    for c in &e.classifications {
                        let Some(code) = c.code.as_ref().filter(|s| !s.is_empty()) else {
                            continue;
                        };
                        let in_list =
                            ctx.allowed_codes.is_empty() || ctx.allowed_codes.contains(code);
                        let matches = pattern.as_ref().map(|re| re.is_match(code)).unwrap_or(true);
                        if !in_list || !matches {
                            failures.push(CheckFailure::element(
                                i,
                                format!("{} classification code '{code}' is not in the configured classification system", describe_elem(e)),
                                match (&pattern, ctx.allowed_codes.is_empty()) {
                                    (Some(re), true) => format!("code matching /{}/", re.as_str()),
                                    (Some(re), false) => format!("code in the allowed list and matching /{}/", re.as_str()),
                                    (None, _) => format!("one of {} allowed codes", ctx.allowed_codes.len()),
                                },
                                code.clone(),
                            ));
                        }
                    }
                }
            }
            CheckKind::NameMatches { pattern } => {
                let explicit = match pattern {
                    Some(p) => {
                        let s = p.resolve(cfg)?.as_str().unwrap_or_default().to_string();
                        if s.is_empty() {
                            None
                        } else {
                            Some(Regex::new(&s).map_err(|e| {
                                CoreError::RulePack(format!("invalid pattern '{s}': {e}"))
                            })?)
                        }
                    }
                    None => None,
                };
                let mut per_type: Vec<(String, Regex)> = Vec::new();
                for (t, p) in &cfg.naming.patterns {
                    per_type.push((
                        t.clone(),
                        Regex::new(p).map_err(|e| {
                            CoreError::Config(format!("naming.patterns['{t}']: {e}"))
                        })?,
                    ));
                }
                if explicit.is_none() && per_type.is_empty() {
                    return Ok(CheckOutcome::Skipped {
                        reason: "no naming patterns configured (naming.patterns)".into(),
                    });
                }
                for &i in ctx.scope {
                    let e = &els[i];
                    let name = name_of(e);
                    let re = explicit
                        .as_ref()
                        .or_else(|| per_type.iter().find(|(t, _)| e.is_a(t)).map(|(_, r)| r));
                    let Some(re) = re else { continue };
                    if !re.is_match(&name) {
                        failures.push(CheckFailure::element(
                            i,
                            format!(
                                "{} name '{}' does not follow the naming convention",
                                e.ifc_type, name
                            ),
                            format!("name matching /{}/", re.as_str()),
                            if name.is_empty() {
                                "<empty>".into()
                            } else {
                                name.clone()
                            },
                        ));
                    }
                }
            }
            CheckKind::QuantityPositive { quantity, min } => {
                let min_v = match min {
                    Some(p) => p.as_f64(cfg)?,
                    None => 0.0,
                };
                for &i in ctx.scope {
                    let e = &els[i];
                    let (label, unit, val) = match quantity {
                        QuantityKind::Volume => ("volume", "m³", e.volume_m3()),
                        QuantityKind::Area => ("area", "m²", e.area_m2()),
                        QuantityKind::Length => ("length", "m", e.length_m()),
                    };
                    if let Some((v, src)) = val {
                        if v <= min_v {
                            failures.push(CheckFailure::element(
                                i,
                                format!(
                                    "{} has {label} {} {unit} (from {})",
                                    describe_elem(e),
                                    crate::model::format_number(v),
                                    src.label()
                                ),
                                format!("{label} > {} {unit}", crate::model::format_number(min_v)),
                                format!("{} {unit}", crate::model::format_number(v)),
                            ));
                        }
                    }
                }
            }
            CheckKind::TypesPresent { ifc_types } => {
                let wanted = ifc_types.as_string_list(cfg)?;
                for t in wanted {
                    let n = els.iter().filter(|e| e.is_a(&t)).count();
                    if n == 0 {
                        failures.push(CheckFailure::model(
                            format!("No {t} elements found in the model"),
                            format!("at least one {t}"),
                            "0",
                        ));
                    }
                }
            }
            CheckKind::SchemaAllowed => {
                let allowed = &cfg.project.ifc_versions;
                if !allowed.is_empty() {
                    let schema = ctx.model.info.schema.to_ascii_uppercase();
                    let ok = allowed
                        .iter()
                        .any(|a| normalise_schema(a) == normalise_schema(&schema));
                    if !ok {
                        failures.push(CheckFailure::model(
                            format!(
                                "File schema {} is not accepted by the project",
                                ctx.model.info.schema
                            ),
                            format!("one of {}", allowed.join(", ")),
                            ctx.model.info.schema.clone(),
                        ));
                    }
                }
            }
        }
        Ok(CheckOutcome::Ran { failures })
    }
}

/// "IFC4X3_ADD2" / "IFC4.3" / "ifc4x3" -> "IFC4X3"; "IFC2x3" -> "IFC2X3"; "IFC4" -> "IFC4".
pub fn normalise_schema(s: &str) -> String {
    let up = s.to_ascii_uppercase().replace('.', "X");
    let base = up.split('_').next().unwrap_or("").to_string();
    match base.as_str() {
        "IFC4X0" | "IFC4X1" | "IFC4X2" => "IFC4".to_string(),
        _ => base,
    }
}

fn type_matches(v: &PropValue, dt: &str) -> bool {
    match dt.to_ascii_lowercase().as_str() {
        "string" | "text" | "label" => matches!(v, PropValue::Text(_)),
        "number" | "real" | "integer" | "float" => matches!(v, PropValue::Number(_)),
        "boolean" | "bool" | "logical" => matches!(v, PropValue::Bool(_)),
        _ => true,
    }
}

fn list_or_none<'a>(it: impl Iterator<Item = &'a String>) -> String {
    let v: Vec<&str> = it.map(|s| s.as_str()).collect();
    if v.is_empty() {
        "none".to_string()
    } else {
        v.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_id_validation() {
        assert!(is_valid_global_id("0fqX614OH1YO1Njdxms2$Q"));
        assert!(!is_valid_global_id("short"));
        assert!(!is_valid_global_id("9fqX614OH1YO1Njdxms2$Q"));
        assert!(!is_valid_global_id("0fqX614OH1YO1Njdxms2#Q"));
    }

    #[test]
    fn schema_normalisation() {
        assert_eq!(normalise_schema("IFC2x3"), "IFC2X3");
        assert_eq!(normalise_schema("IFC4X3_ADD2"), "IFC4X3");
        assert_eq!(normalise_schema("IFC4.3"), "IFC4X3");
        assert_eq!(normalise_schema("ifc4"), "IFC4");
        assert_eq!(normalise_schema("IFC4X1"), "IFC4");
    }

    #[test]
    fn param_resolves_config_reference() {
        let cfg = Config::default();
        let p = Param::Text("$config.tolerances.far_from_origin_m".into());
        assert_eq!(p.as_f64(&cfg).unwrap(), 10_000.0);
        assert!(Param::Text("$config.nope".into()).as_f64(&cfg).is_err());
        assert_eq!(Param::Number(3.0).as_f64(&cfg).unwrap(), 3.0);
    }
}
