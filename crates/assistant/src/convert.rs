//! Documentation → knowledge YAML.
//!
//! Turns project documentation (BIM execution plans, employer's information
//! requirements, naming conventions, ...) into [`Knowledge`] chunks the
//! assistant can retrieve. Pure text extraction, no model involved.
//!
//! STUB: to be implemented. Contract:
//! * `convert_paths(paths)` accepts files and directories (recursed) with
//!   extensions .md .markdown .txt .yaml .yml .json .docx .pdf; others are
//!   skipped and listed in `ConvertReport.skipped`.
//! * .md/.txt: `knowledge::chunk_markdown`. .yaml/.yml: if it already is a
//!   Knowledge file, merge; else chunk the YAML text by top-level keys.
//!   .json: pretty-print and chunk. .docx: unzip `word/document.xml`, take
//!   `w:p` paragraphs (quick-xml), headings from `w:pStyle` Heading1..3.
//!   .pdf: `pdf_extract::extract_text`, paragraphs split on blank lines.
//! * Every chunk gets `source` = relative path, `keywords` = top 8 distinct
//!   capitalised or underscored tokens (identifiers like Pset_WallCommon).
//! * Deterministic order (sorted paths).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AssistantResult;
use crate::knowledge::Knowledge;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConvertReport {
    pub knowledge: Knowledge,
    pub files_read: Vec<String>,
    pub skipped: Vec<String>,
}

pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "yaml", "yml", "json", "docx", "pdf",
];

/// Convert files/directories into knowledge.
pub fn convert_paths(paths: &[PathBuf]) -> AssistantResult<ConvertReport> {
    let _ = paths;
    Err(crate::error::AssistantError::Convert(
        "not implemented yet".into(),
    ))
}

/// Convert and write `knowledge.yaml` next to the outputs.
pub fn convert_to_file(paths: &[PathBuf], out: &Path) -> AssistantResult<ConvertReport> {
    let r = convert_paths(paths)?;
    std::fs::write(out, r.knowledge.to_yaml())?;
    Ok(r)
}
