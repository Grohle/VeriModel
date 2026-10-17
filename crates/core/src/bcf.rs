//! BCF writer (BIM Collaboration Format).
//!
//! STUB: to be implemented. Contract:
//! * `write_bcfzip(report, model, config, out_path) -> BcfStats` writes a
//!   BCF 2.1 (`output.bcf_version == "2.1"`) or BCF 3.0 `.bcfzip`: one topic
//!   per ERROR/WARNING finding, with markup.bcf (title, description, status,
//!   priority, author, creation date, related GlobalId as a component) and a
//!   viewpoint.bcfv (perspective camera framing the element's bounding box
//!   when geometry is available). Deterministic topic GUIDs (uuid v5 from
//!   run-independent data) so re-runs produce identical files except dates.
//! * `write_issues_csv(report, out_path)` writes one CSV row per issue.

use std::path::Path;

use crate::config::Config;
use crate::error::CoreResult;
use crate::model::{Model, RunReport};

/// Numbers reported after writing a BCF file.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BcfStats {
    pub topics: usize,
    pub with_viewpoint: usize,
    pub version: String,
}

/// Write `issues.bcfzip`.
pub fn write_bcfzip(report: &RunReport, model: &Model, config: &Config, out_path: &Path) -> CoreResult<BcfStats> {
    let _ = (report, model, config, out_path);
    Ok(BcfStats::default())
}

/// Write `issues.csv` (one row per ERROR/WARNING finding, Excel friendly).
pub fn write_issues_csv(report: &RunReport, out_path: &Path) -> CoreResult<usize> {
    let _ = (report, out_path);
    Ok(0)
}
