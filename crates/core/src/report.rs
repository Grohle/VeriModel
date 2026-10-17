//! Report writers: report.html, report.json, failures.csv, qto.csv, qto.xlsx, qto.json.
//!
//! STUB: to be implemented. Contract:
//! * `write_report_json(report, path)` — pretty JSON, keys in struct order, no timestamps.
//! * `write_failures_csv(report, path)` — one row per finding:
//!   severity,rule_id,global_id,step_id,ifc_type,name,storey,message,expected,actual,fix_hint,source
//! * `write_qto_csv(report, path)` / `write_qto_xlsx(report, path)` / `write_qto_json`.
//! * `write_report_html(report, strings, path)` — self-contained HTML (inline CSS,
//!   no external assets), summary + per-rule table + QTO tables + failure list +
//!   IDS section + link to BCF. Language from `Strings`.
//! * `write_all(report, config, out_dir) -> Vec<OutputFile>` calls the above.
//! All writers must be deterministic (same report -> same bytes).

use std::path::Path;

use crate::config::Config;
use crate::error::CoreResult;
use crate::i18n::Strings;
use crate::model::RunReport;

/// One produced file, listed on the Export screen.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OutputFile {
    /// File name inside `out/`.
    pub name: String,
    /// Plain-language description key (resolved by the UI's i18n).
    pub kind: String,
    pub bytes: u64,
}

pub fn write_report_json(report: &RunReport, path: &Path) -> CoreResult<()> {
    let text = serde_json::to_string_pretty(report)?;
    std::fs::write(path, text)?;
    Ok(())
}

pub fn write_failures_csv(report: &RunReport, path: &Path) -> CoreResult<()> {
    let _ = (report, path);
    Ok(())
}

pub fn write_qto_csv(report: &RunReport, path: &Path) -> CoreResult<()> {
    let _ = (report, path);
    Ok(())
}

pub fn write_qto_xlsx(report: &RunReport, path: &Path) -> CoreResult<()> {
    let _ = (report, path);
    Ok(())
}

pub fn write_report_html(report: &RunReport, strings: &Strings, path: &Path) -> CoreResult<()> {
    let _ = (report, strings, path);
    Ok(())
}

/// Write every report file into `out_dir` and return the list.
pub fn write_all(report: &RunReport, config: &Config, out_dir: &Path) -> CoreResult<Vec<OutputFile>> {
    let strings = Strings::for_language(config.output.language);
    let mut files = Vec::new();
    let mut add = |name: &str, kind: &str| -> CoreResult<()> {
        let p = out_dir.join(name);
        let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        files.push(OutputFile { name: name.into(), kind: kind.into(), bytes });
        Ok(())
    };
    write_report_json(report, &out_dir.join("report.json"))?;
    add("report.json", "report_json")?;
    write_report_html(report, &strings, &out_dir.join("report.html"))?;
    add("report.html", "report_html")?;
    write_failures_csv(report, &out_dir.join("failures.csv"))?;
    add("failures.csv", "failures_csv")?;
    write_qto_csv(report, &out_dir.join("qto.csv"))?;
    add("qto.csv", "qto_csv")?;
    write_qto_xlsx(report, &out_dir.join("qto.xlsx"))?;
    add("qto.xlsx", "qto_xlsx")?;
    Ok(files)
}
