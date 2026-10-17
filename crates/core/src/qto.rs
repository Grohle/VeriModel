//! Quantity takeoff: aggregation, grouping, cost estimate, hand-check samples.
//!
//! STUB: to be implemented. Contract:
//! * `RateTable::from_csv(path)` reads `code,description,unit,rate,currency,match_type,match_storey,match_classification`.
//! * `compute(model, config, rates) -> QtoReport` groups non-spatial elements by
//!   `config.qto.group_by` (type | storey | classification | material |
//!   predefined_type | type_name), sums volume/area/length/weight using
//!   `Element::volume_m3()` etc. (base quantity first, geometry fallback),
//!   records per-source counts, matches a rate per group, picks
//!   `config.qto.sample_count` deterministic samples (first N by GlobalId with
//!   any quantity) and writes a plain-language `method` string. Rows sorted by
//!   group key. Totals are sums of rows.

use std::path::Path;

use crate::config::Config;
use crate::error::CoreResult;
use crate::model::{Model, QtoReport};

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
    pub fn from_csv(path: &Path) -> CoreResult<RateTable> {
        let _ = path;
        Ok(RateTable::default())
    }
}

/// Compute the quantity takeoff.
pub fn compute(model: &Model, config: &Config, rates: Option<&RateTable>) -> CoreResult<QtoReport> {
    let _ = (model, config, rates);
    Ok(QtoReport { group_by: config.qto.group_by.clone(), method: "stub".into(), ..Default::default() })
}
