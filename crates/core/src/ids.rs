//! IDS (Information Delivery Specification) integration.
//!
//! The IDS engine (IfcTester, driven by the reader) returns facts in
//! [`IdsResults`]. This module turns them into rules and findings so IDS
//! requirements show up in the same tables as rule-pack checks, quoting the
//! requirement verbatim.
//!
//! STUB: to be implemented. Contract:
//! * `evaluate(results) -> IdsEvaluation` with one `RuleResult` per
//!   (specification, requirement) pair, rule ids `IDS-<spec#>-<req#>`,
//!   severity ERROR for required specifications, WARNING for optional ones;
//!   one `Finding` per failed entity with `source = RuleSource::Ids {..}` and
//!   `expected` = the quoted requirement text.
//! * `read_metadata(path) -> IdsMetadata` parsing the `.ids` XML header
//!   (title, author, version, date) with quick-xml, used in run.json/report.

use crate::model::{Finding, IdsReport, IdsResults, RuleResult};

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

/// Convert IDS engine results into rules and findings.
pub fn evaluate(results: &IdsResults) -> IdsEvaluation {
    let _ = results;
    IdsEvaluation::default()
}

/// Parse the header of an `.ids` file.
pub fn read_metadata(path: &std::path::Path) -> crate::CoreResult<IdsMetadata> {
    let _ = path;
    Ok(IdsMetadata::default())
}
