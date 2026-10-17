//! Quantity takeoff: aggregation, grouping, cost estimate, hand-check samples.
//!
//! * [`RateTable::from_csv`] reads a unit-rate table with the columns
//!   `code,description,unit,rate,currency,match_type,match_storey,match_classification`
//!   (header row required, column names case-insensitive, column order free).
//! * [`compute`] groups the takeoff items of a [`Model`] by `config.qto.group_by`
//!   (`type | storey | classification | material | predefined_type | type_name`),
//!   sums volume / area / length / weight with the [`Element`] helpers (base
//!   quantity first, geometry fallback), records how many elements took each
//!   quantity from which source, matches the first applicable unit rate per
//!   group, picks deterministic hand-check samples and writes a plain-language
//!   `method` text so every total can be audited.
//!
//! Takeoff items are every non-spatial product except abstract non-physical
//! families (feature elements such as openings, virtual elements, annotations
//! and grids). No concrete building type is hardcoded.

use std::collections::BTreeMap;
use std::path::Path;

use crate::config::Config;
use crate::error::{CoreError, CoreResult};
use crate::model::{CostLine, Element, Model, QtoReport, QtoRow, QtoSample, QuantitySource};

/// Abstract IFC families that are never takeoff items (they are not physical
/// products: openings/voids, virtual boundaries, annotations, grids).
const EXCLUDED_FAMILIES: [&str; 4] = [
    "IfcFeatureElement",
    "IfcVirtualElement",
    "IfcAnnotation",
    "IfcGrid",
];

/// Units a rate may be expressed in.
const RATE_UNITS: [&str; 5] = ["m3", "m2", "m", "kg", "nr"];

/// One unit rate from rates.csv.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct Rate {
    pub code: String,
    pub description: String,
    /// "m3" | "m2" | "m" | "kg" | "nr"
    pub unit: String,
    pub rate: f64,
    pub currency: String,
    /// IFC type (or ancestor) this rate applies to.
    pub match_type: Option<String>,
    pub match_storey: Option<String>,
    pub match_classification: Option<String>,
}

/// Loaded rate table.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RateTable {
    pub source: String,
    pub rates: Vec<Rate>,
}

impl RateTable {
    /// Read a unit-rate CSV. Errors are plain language and name the file and line.
    pub fn from_csv(path: &Path) -> CoreResult<RateTable> {
        let source = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let bytes = std::fs::read(path).map_err(|e| {
            CoreError::Qto(format!(
                "cannot read the unit rates file {}: {e}",
                path.display()
            ))
        })?;
        Self::from_reader(&source, bytes.as_slice())
    }

    /// Parse a unit-rate table from any reader; `source` names it in messages and cost lines.
    pub fn from_reader<R: std::io::Read>(source: &str, reader: R) -> CoreResult<RateTable> {
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .trim(csv::Trim::All)
            .flexible(true)
            .from_reader(reader);
        let headers = rdr
            .headers()
            .map_err(|e| {
                CoreError::Qto(format!(
                    "the unit rates file {source} could not be read as CSV: {e}"
                ))
            })?
            .clone();
        let col = |name: &str| {
            headers.iter().position(|h| {
                h.trim_start_matches('\u{feff}')
                    .trim()
                    .eq_ignore_ascii_case(name)
            })
        };
        let required = ["code", "description", "unit", "rate"];
        let missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|c| col(c).is_none())
            .collect();
        if !missing.is_empty() {
            return Err(CoreError::Qto(format!(
                "the unit rates file {source} is missing the column(s) {}. The first line must be a header with: \
                 code, description, unit, rate, currency, match_type, match_storey, match_classification",
                missing.join(", ")
            )));
        }
        let (c_code, c_desc, c_unit, c_rate) = (
            col("code").unwrap(),
            col("description").unwrap(),
            col("unit").unwrap(),
            col("rate").unwrap(),
        );
        let (c_cur, c_type, c_storey, c_class) = (
            col("currency"),
            col("match_type"),
            col("match_storey"),
            col("match_classification"),
        );

        let mut rates = Vec::new();
        for (i, rec) in rdr.records().enumerate() {
            let line = i + 2; // 1-based, header is line 1
            let rec = rec.map_err(|e| {
                CoreError::Qto(format!("the unit rates file {source}, line {line}: {e}"))
            })?;
            if rec.iter().all(|f| f.trim().is_empty()) {
                continue;
            }
            let get = |c: usize| rec.get(c).unwrap_or("").trim().to_string();
            let opt = |c: Option<usize>| c.map(get).filter(|s| !s.is_empty());
            let code = get(c_code);
            if code.is_empty() {
                return Err(CoreError::Qto(format!(
                    "the unit rates file {source}, line {line}: the code is empty"
                )));
            }
            let unit = get(c_unit).to_ascii_lowercase();
            if !RATE_UNITS.contains(&unit.as_str()) {
                return Err(CoreError::Qto(format!(
                    "the unit rates file {source}, line {line} (code {code}): unit '{}' is not one of m3, m2, m, kg, nr",
                    get(c_unit)
                )));
            }
            let rate_text = get(c_rate);
            let rate: f64 = rate_text.parse().ok().filter(|r: &f64| r.is_finite()).ok_or_else(|| {
                CoreError::Qto(format!(
                    "the unit rates file {source}, line {line} (code {code}): rate '{rate_text}' is not a number \
                     (use a dot as decimal separator, no currency symbol)"
                ))
            })?;
            rates.push(Rate {
                code,
                description: get(c_desc),
                unit,
                rate,
                currency: opt(c_cur).unwrap_or_default(),
                match_type: opt(c_type),
                match_storey: opt(c_storey),
                match_classification: opt(c_class),
            });
        }
        Ok(RateTable {
            source: source.to_string(),
            rates,
        })
    }
}

/// True when the element is a takeoff item.
pub fn is_takeoff_item(e: &Element) -> bool {
    !e.is_spatial() && !EXCLUDED_FAMILIES.iter().any(|f| e.is_a(f))
}

/// The value of one grouping key for an element.
fn group_value(e: &Element, key: &str) -> CoreResult<String> {
    let non_empty = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    Ok(match key {
        "type" => e.ifc_type.clone(),
        "storey" => non_empty(&e.storey).unwrap_or_else(|| "(no storey)".into()),
        "classification" => first_classification(e).unwrap_or_else(|| "(unclassified)".into()),
        "material" => {
            let mats: Vec<&str> = e.materials.iter().map(|m| m.trim()).filter(|m| !m.is_empty()).collect();
            if mats.is_empty() {
                "(no material)".into()
            } else {
                mats.join(" | ")
            }
        }
        "predefined_type" => non_empty(&e.predefined_type).unwrap_or_else(|| "(none)".into()),
        "type_name" => non_empty(&e.type_name).unwrap_or_else(|| "(no type)".into()),
        other => {
            return Err(CoreError::Qto(format!(
                "qto.group_by contains '{other}', expected one of type, storey, classification, material, predefined_type, type_name"
            )))
        }
    })
}

fn first_classification(e: &Element) -> Option<String> {
    e.classifications.iter().find_map(|c| {
        c.code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    })
}

fn key_phrase(key: &str) -> &'static str {
    match key {
        "type" => "IFC entity type",
        "storey" => "building storey",
        "classification" => "first classification code",
        "material" => "material (all materials of the element joined with ' | ')",
        "predefined_type" => "predefined type",
        "type_name" => "type object name",
        _ => "unknown key",
    }
}

/// Whether a rate applies to the elements of one group.
///
/// `match_type` uses `Element::is_a` (type or any ancestor); `match_storey`
/// is an exact storey name; `match_classification` is a prefix of the first
/// classification code. Every element of the group must satisfy each
/// non-empty condition, so a rate never prices elements it was not written
/// for (when the group key already fixes the attribute this is the same as
/// testing the group's first element).
fn rate_matches(rate: &Rate, elements: &[&Element]) -> bool {
    if elements.is_empty() {
        return false;
    }
    if let Some(t) = &rate.match_type {
        if !elements.iter().all(|e| e.is_a(t)) {
            return false;
        }
    }
    if let Some(s) = &rate.match_storey {
        if !elements
            .iter()
            .all(|e| e.storey.as_deref().map(str::trim) == Some(s.as_str()))
        {
            return false;
        }
    }
    if let Some(p) = &rate.match_classification {
        if !elements
            .iter()
            .all(|e| first_classification(e).is_some_and(|c| c.starts_with(p.as_str())))
        {
            return false;
        }
    }
    true
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn bump(map: &mut BTreeMap<String, usize>, src: QuantitySource) {
    *map.entry(src.label().to_string()).or_insert(0) += 1;
}

fn source_of(q: Option<(f64, QuantitySource)>) -> (f64, QuantitySource) {
    q.unwrap_or((0.0, QuantitySource::None))
}

fn has_any_quantity(e: &Element) -> bool {
    e.volume_m3().is_some()
        || e.area_m2().is_some()
        || e.length_m().is_some()
        || e.weight_kg().is_some()
}

/// Compute the quantity takeoff.
pub fn compute(model: &Model, config: &Config, rates: Option<&RateTable>) -> CoreResult<QtoReport> {
    let group_by = config.qto.group_by.clone();
    for k in &group_by {
        group_value(&Element::default(), k)?; // validates the key
    }

    let mut items: Vec<&Element> = model
        .elements
        .iter()
        .filter(|e| is_takeoff_item(e))
        .collect();
    items.sort_by(|a, b| {
        a.global_id
            .cmp(&b.global_id)
            .then(a.step_id.cmp(&b.step_id))
    });
    let excluded = model.elements.len() - items.len();

    let mut groups: BTreeMap<Vec<String>, Vec<&Element>> = BTreeMap::new();
    for e in &items {
        let key = group_by
            .iter()
            .map(|k| group_value(e, k))
            .collect::<CoreResult<Vec<_>>>()?;
        groups.entry(key).or_default().push(e);
    }

    let mut rows = Vec::with_capacity(groups.len());
    let mut src_totals: [BTreeMap<String, usize>; 3] = Default::default();
    let mut weight_count = 0usize;
    for (group, elems) in &groups {
        let mut row = QtoRow {
            group: group.clone(),
            count: elems.len(),
            ..Default::default()
        };
        for e in elems {
            let (v, vs) = source_of(e.volume_m3());
            let (a, as_) = source_of(e.area_m2());
            let (l, ls) = source_of(e.length_m());
            row.volume_m3 += v;
            row.area_m2 += a;
            row.length_m += l;
            if let Some((w, _)) = e.weight_kg() {
                row.weight_kg += w;
                weight_count += 1;
            }
            bump(&mut row.volume_sources, vs);
            bump(&mut row.area_sources, as_);
            bump(&mut row.length_sources, ls);
            bump(&mut src_totals[0], vs);
            bump(&mut src_totals[1], as_);
            bump(&mut src_totals[2], ls);
        }
        if let Some(table) = rates {
            if let Some(rate) = table.rates.iter().find(|r| rate_matches(r, elems)) {
                let quantity = match rate.unit.as_str() {
                    "m3" => row.volume_m3,
                    "m2" => row.area_m2,
                    "m" => row.length_m,
                    "kg" => row.weight_kg,
                    _ => row.count as f64,
                };
                let currency = if rate.currency.is_empty() {
                    config.qto.currency.clone()
                } else {
                    rate.currency.clone()
                };
                row.cost = Some(CostLine {
                    rate_code: rate.code.clone(),
                    rate_description: rate.description.clone(),
                    unit: rate.unit.clone(),
                    rate: rate.rate,
                    quantity,
                    amount: round2(quantity * rate.rate),
                    currency,
                    source: table.source.clone(),
                });
            }
        }
        rows.push(row);
    }

    let costs: Vec<&CostLine> = rows.iter().filter_map(|r| r.cost.as_ref()).collect();
    let mut currencies: Vec<String> = costs.iter().map(|c| c.currency.clone()).collect();
    currencies.sort_unstable();
    currencies.dedup();
    let mixed_currency = currencies.len() > 1;
    let (total_cost, currency) = match currencies.len() {
        0 => (None, None),
        1 => (
            Some(round2(costs.iter().map(|c| c.amount).sum())),
            Some(currencies[0].clone()),
        ),
        _ => (None, None),
    };

    let without = items.iter().filter(|e| !has_any_quantity(e)).count();

    let samples: Vec<QtoSample> = items
        .iter()
        .filter(|e| has_any_quantity(e))
        .take(config.qto.sample_count)
        .map(|e| {
            let (v, vs) = split(e.volume_m3());
            let (a, as_) = split(e.area_m2());
            let (l, ls) = split(e.length_m());
            QtoSample {
                global_id: e.global_id.clone(),
                ifc_type: e.ifc_type.clone(),
                name: e.name.clone(),
                storey: e.storey.clone(),
                volume_m3: v,
                volume_source: vs,
                area_m2: a,
                area_source: as_,
                length_m: l,
                length_source: ls,
                quantity_sets: e.quantities.keys().cloned().collect(),
            }
        })
        .collect();

    let mut report = QtoReport {
        group_by: group_by.clone(),
        total_count: rows.iter().map(|r| r.count).sum(),
        total_volume_m3: rows.iter().map(|r| r.volume_m3).sum(),
        total_area_m2: rows.iter().map(|r| r.area_m2).sum(),
        total_length_m: rows.iter().map(|r| r.length_m).sum(),
        total_weight_kg: rows.iter().map(|r| r.weight_kg).sum(),
        total_cost,
        currency,
        elements_without_quantities: without,
        method: String::new(),
        samples,
        rows,
    };
    report.method = method_text(
        &report,
        excluded,
        &src_totals,
        weight_count,
        rates,
        mixed_currency,
    );
    Ok(report)
}

fn split(q: Option<(f64, QuantitySource)>) -> (Option<f64>, QuantitySource) {
    match q {
        Some((v, s)) => (Some(v), s),
        None => (None, QuantitySource::None),
    }
}

fn sources_phrase(m: &BTreeMap<String, usize>) -> String {
    let n = |k: &str| m.get(k).copied().unwrap_or(0);
    format!(
        "{} from base quantities, {} from geometry, {} not available",
        n("base_quantity"),
        n("geometry"),
        n("none")
    )
}

fn method_text(
    q: &QtoReport,
    excluded: usize,
    src: &[BTreeMap<String, usize>; 3],
    weight_count: usize,
    rates: Option<&RateTable>,
    mixed_currency: bool,
) -> String {
    let keys = q
        .group_by
        .iter()
        .map(|k| key_phrase(k))
        .collect::<Vec<_>>()
        .join(", then by ");
    let mut s = String::new();
    s.push_str(&format!(
        "{} element(s) were measured and grouped by {}; {} groups resulted. ",
        q.total_count,
        if keys.is_empty() {
            "nothing (one single group)".to_string()
        } else {
            keys
        },
        q.rows.len()
    ));
    s.push_str(&format!(
        "Spatial elements (sites, buildings, storeys, spaces) and non-physical elements (openings and other feature \
         elements, virtual elements, annotations, grids) are not takeoff items; {excluded} such entries were left out. "
    ));
    s.push_str(
        "For each element, volume, area, length and weight were taken first from the base quantities stored in the IFC \
         file (IfcElementQuantity, e.g. NetVolume, NetArea, Length, NetWeight). When a base quantity was missing, volume \
         and area were computed from the tessellated geometry and length was taken as the longest side of the bounding \
         box; weight is only reported when the file provides it. ",
    );
    s.push_str(
        "All values are in SI units, converted from the units declared in the file: cubic metres (m³), square metres \
         (m²), metres (m) and kilograms (kg). ",
    );
    s.push_str(&format!(
        "Volume: {}. Area: {}. Length: {}. Weight was available for {} element(s). ",
        sources_phrase(&src[0]),
        sources_phrase(&src[1]),
        sources_phrase(&src[2]),
        weight_count
    ));
    s.push_str(&format!(
        "{} element(s) had no quantity at all; they are counted in their group but add nothing to the totals. ",
        q.elements_without_quantities
    ));
    match rates {
        None => s.push_str("No unit rates file was given, so no cost was estimated."),
        Some(t) => {
            let matched = q.rows.iter().filter(|r| r.cost.is_some()).count();
            s.push_str(&format!(
                "Cost was estimated with the unit rates in {} ({} rates): for each group the first rate, in file order, \
                 whose match columns all apply (match_type: the IFC type or one of its supertypes; match_storey: the exact \
                 storey name; match_classification: a prefix of the classification code; an empty column matches \
                 anything) was used. Amount = group quantity in the rate's unit (nr = element count) × rate, rounded to 2 \
                 decimals. {} of {} groups matched a rate; groups without a rate have no cost. ",
                t.source,
                t.rates.len(),
                matched,
                q.rows.len()
            ));
            if mixed_currency {
                s.push_str(
                    "The matched rates use different currencies, so no grand total was computed.",
                );
            } else if let (Some(total), Some(cur)) = (q.total_cost, &q.currency) {
                s.push_str(&format!("Total estimated cost: {total:.2} {cur}."));
            } else {
                s.push_str("No rate matched, so no total cost was computed.");
            }
        }
    }
    s.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Classification, Geometry};

    fn el(id: &str, ty: &str, storey: Option<&str>) -> Element {
        Element {
            global_id: id.into(),
            ifc_type: ty.into(),
            ancestors: vec![
                ty.into(),
                "IfcBuiltElement".into(),
                "IfcElement".into(),
                "IfcProduct".into(),
            ],
            storey: storey.map(String::from),
            ..Default::default()
        }
    }

    fn with_qty(mut e: Element, name: &str, v: f64) -> Element {
        e.quantities
            .entry("Qto_Base".into())
            .or_default()
            .insert(name.into(), v);
        e
    }

    fn with_geom(mut e: Element, vol: f64, area: f64) -> Element {
        e.geometry = Some(Geometry {
            bbox_min: [0.0, 0.0, 0.0],
            bbox_max: [4.0, 0.2, 3.0],
            volume_m3: Some(vol),
            area_m2: Some(area),
            triangle_count: 12,
        });
        e
    }

    fn model() -> Model {
        let mut m = Model {
            elements: vec![
                with_qty(
                    with_qty(el("A1", "IfcWall", Some("L1")), "NetVolume", 2.0),
                    "NetSideArea",
                    10.0,
                ),
                with_geom(el("A2", "IfcWall", Some("L1")), 3.0, 12.0),
                with_qty(el("A3", "IfcWall", Some("L2")), "NetVolume", 1.5),
                {
                    let mut s = with_qty(el("B1", "IfcSlab", Some("L1")), "NetVolume", 5.0);
                    s.classifications.push(Classification {
                        code: Some("Ss_30_12".into()),
                        ..Default::default()
                    });
                    s
                },
                el("C1", "IfcColumn", None), // no quantity at all
                Element {
                    global_id: "S1".into(),
                    ifc_type: "IfcBuildingStorey".into(),
                    ancestors: vec![
                        "IfcBuildingStorey".into(),
                        "IfcSpatialStructureElement".into(),
                    ],
                    ..Default::default()
                },
                Element {
                    global_id: "O1".into(),
                    ifc_type: "IfcOpeningElement".into(),
                    ancestors: vec![
                        "IfcOpeningElement".into(),
                        "IfcFeatureElementSubtraction".into(),
                        "IfcFeatureElement".into(),
                    ],
                    geometry: Some(Geometry {
                        volume_m3: Some(9.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        m.sort_elements();
        m
    }

    #[test]
    fn groups_by_type_and_storey_with_totals_and_sources() {
        let cfg = Config::default();
        let q = compute(&model(), &cfg, None).unwrap();
        let groups: Vec<Vec<String>> = q.rows.iter().map(|r| r.group.clone()).collect();
        assert_eq!(
            groups,
            vec![
                vec!["IfcColumn".to_string(), "(no storey)".to_string()],
                vec!["IfcSlab".into(), "L1".into()],
                vec!["IfcWall".into(), "L1".into()],
                vec!["IfcWall".into(), "L2".into()],
            ]
        );
        let wall_l1 = &q.rows[2];
        assert_eq!(wall_l1.count, 2);
        assert!((wall_l1.volume_m3 - 5.0).abs() < 1e-9);
        assert!((wall_l1.area_m2 - 22.0).abs() < 1e-9);
        assert_eq!(wall_l1.volume_sources.get("base_quantity"), Some(&1));
        assert_eq!(wall_l1.volume_sources.get("geometry"), Some(&1));
        assert_eq!(q.total_count, 5, "storey and opening are excluded");
        assert!((q.total_volume_m3 - 11.5).abs() < 1e-9);
        assert_eq!(q.elements_without_quantities, 1);
        assert_eq!(q.rows[0].volume_sources.get("none"), Some(&1));
        assert!(q.total_cost.is_none());
        assert!(q.method.contains("base quantities"));
        assert!(q.method.contains("1 element(s) had no quantity"));
    }

    #[test]
    fn rate_matching_first_match_wins_and_non_matching_is_ignored() {
        let csv =
            "Code,Description,Unit,Rate,Currency,match_type,match_storey,match_classification\n\
                   R-DOOR,Doors,nr,300,,IfcDoor,,\n\
                   R-WALL-L1,Walls level 1,m3,100,USD,IfcWall,L1,\n\
                   R-WALL,Walls,m3,90,,IfcBuiltElement,,\n\
                   R-SLAB,Slabs by class,m2,10,,,,Ss_30\n";
        let table = RateTable::from_reader("rates.csv", csv.as_bytes()).unwrap();
        assert_eq!(table.rates.len(), 4);
        let cfg = Config::default();
        let q = compute(&model(), &cfg, Some(&table)).unwrap();
        let by = |t: &str, s: &str| {
            q.rows
                .iter()
                .find(|r| r.group == vec![t.to_string(), s.to_string()])
                .unwrap()
        };
        let c = by("IfcWall", "L1").cost.as_ref().unwrap();
        assert_eq!(c.rate_code, "R-WALL-L1");
        assert_eq!(c.currency, "USD");
        assert_eq!(c.amount, 500.0);
        assert_eq!(c.source, "rates.csv");
        let c2 = by("IfcWall", "L2").cost.as_ref().unwrap();
        assert_eq!(c2.rate_code, "R-WALL");
        assert_eq!(c2.currency, "EUR", "falls back to config currency");
        assert_eq!(c2.amount, 135.0);
        // Slab has no area, classification prefix matches but the ancestor rate R-WALL comes first.
        assert_eq!(
            by("IfcSlab", "L1").cost.as_ref().unwrap().rate_code,
            "R-WALL"
        );
        // Mixed currencies -> no grand total.
        assert!(q.total_cost.is_none());

        let csv2 =
            "code,description,unit,rate,currency,match_type,match_storey,match_classification\n\
                    R-DOOR,Doors,nr,300,EUR,IfcDoor,,\n\
                    R-SLAB,Slabs,m3,20,EUR,,,Ss_30\n";
        let t2 = RateTable::from_reader("r2.csv", csv2.as_bytes()).unwrap();
        let q2 = compute(&model(), &cfg, Some(&t2)).unwrap();
        assert_eq!(q2.rows.iter().filter(|r| r.cost.is_some()).count(), 1);
        assert_eq!(q2.total_cost, Some(100.0));
        assert_eq!(q2.currency.as_deref(), Some("EUR"));
        assert!(q2.method.contains("1 of 4 groups matched"));
    }

    #[test]
    fn samples_are_deterministic_and_skip_elements_without_quantities() {
        let mut cfg = Config::default();
        cfg.qto.sample_count = 3;
        let a = compute(&model(), &cfg, None).unwrap();
        let b = compute(&model(), &cfg, None).unwrap();
        assert_eq!(a, b);
        let ids: Vec<&str> = a.samples.iter().map(|s| s.global_id.as_str()).collect();
        assert_eq!(ids, vec!["A1", "A2", "A3"]);
        assert_eq!(a.samples[0].quantity_sets, vec!["Qto_Base".to_string()]);
        assert_eq!(a.samples[1].volume_source, QuantitySource::Geometry);
        cfg.qto.sample_count = 10;
        let c = compute(&model(), &cfg, None).unwrap();
        assert!(c
            .samples
            .iter()
            .all(|s| s.global_id != "C1" && s.global_id != "O1"));
    }

    #[test]
    fn other_group_keys_use_placeholders() {
        let mut cfg = Config::default();
        cfg.qto.group_by = vec![
            "classification".into(),
            "material".into(),
            "predefined_type".into(),
            "type_name".into(),
        ];
        let q = compute(&model(), &cfg, None).unwrap();
        assert!(q
            .rows
            .iter()
            .any(|r| r.group == vec!["(unclassified)", "(no material)", "(none)", "(no type)"]));
        assert!(q.rows.iter().any(|r| r.group[0] == "Ss_30_12"));
    }

    #[test]
    fn missing_column_is_a_plain_language_qto_error() {
        let err =
            RateTable::from_reader("rates.csv", "code,unit,rate\nX,m3,1\n".as_bytes()).unwrap_err();
        assert!(matches!(err, CoreError::Qto(_)));
        let msg = err.to_string();
        assert!(msg.contains("missing the column(s) description"), "{msg}");
        assert!(msg.contains("rates.csv"));
    }

    #[test]
    fn bad_rate_and_unit_are_reported_with_line_numbers() {
        let bad_rate = "code,description,unit,rate\nX,d,m3,12€\n";
        let e = RateTable::from_reader("r.csv", bad_rate.as_bytes())
            .unwrap_err()
            .to_string();
        assert!(e.contains("line 2") && e.contains("not a number"), "{e}");
        let bad_unit = "code,description,unit,rate\nX,d,m3,1\nY,d,ft,1\n";
        let e = RateTable::from_reader("r.csv", bad_unit.as_bytes())
            .unwrap_err()
            .to_string();
        assert!(e.contains("line 3") && e.contains("'ft'"), "{e}");
    }

    #[test]
    fn from_csv_reads_a_file_and_names_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("my_rates.csv");
        std::fs::write(&p, "CODE,DESCRIPTION,UNIT,RATE\nA,Any,nr,2.5\n").unwrap();
        let t = RateTable::from_csv(&p).unwrap();
        assert_eq!(t.source, "my_rates.csv");
        assert_eq!(t.rates[0].rate, 2.5);
        assert!(t.rates[0].match_type.is_none());
        let missing = RateTable::from_csv(&dir.path().join("nope.csv")).unwrap_err();
        assert!(matches!(missing, CoreError::Qto(_)));
    }
}
