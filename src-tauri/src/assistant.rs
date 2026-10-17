//! Plain, Tauri-free helpers behind the assistant commands, so they can be unit-tested
//! without launching a window.
//!
//! The assistant is a read-only copilot: nothing here touches verdicts, scores or quantities.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use verimodel_assistant::model::ModelStatus;
use verimodel_assistant::{Assistant, AssistantError, Knowledge};
use verimodel_core::model::RunReport;

use crate::error::AppError;

/// Event name the UI listens to for model download progress: `{ done, total }`.
pub const DOWNLOAD_PROGRESS_EVENT: &str = "assistant://download-progress";

/// What the UI receives from `app_info`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

pub fn app_info() -> AppInfo {
    AppInfo {
        name: verimodel_core::TOOL_NAME.to_string(),
        version: verimodel_core::VERSION.to_string(),
    }
}

/// `ModelStatus` plus whether the model is currently loaded in memory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssistantStatus {
    #[serde(flatten)]
    pub status: ModelStatus,
    pub loaded: bool,
}

/// Result of importing documentation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportSummary {
    pub files_read: Vec<String>,
    pub skipped: Vec<String>,
    pub chunks_added: usize,
    /// The knowledge converted by this import, as `knowledge.yaml` text (for "Save knowledge.yaml").
    pub knowledge_yaml: String,
}

/// The three sources of what the assistant can retrieve, kept apart so that a new run
/// replaces the previous report's chunks without touching the built-in docs or the
/// documentation the user imported.
#[derive(Debug, Clone, Default)]
pub struct KnowledgeParts {
    builtin: Knowledge,
    imported: Knowledge,
    report: Knowledge,
}

impl KnowledgeParts {
    pub fn new(builtin: Knowledge) -> Self {
        Self {
            builtin,
            imported: Knowledge::default(),
            report: Knowledge::default(),
        }
    }

    /// Replace the report chunks with those of `report`.
    pub fn set_report(&mut self, report: &RunReport) {
        let mut k = Knowledge::default();
        k.add_report(report);
        self.report = k;
    }

    /// Add imported documentation. Returns the number of chunks added.
    pub fn add_imported(&mut self, k: Knowledge) -> usize {
        let n = k.chunks.len();
        self.imported.extend(k);
        n
    }

    /// Imported documentation only, as `knowledge.yaml` text.
    pub fn imported_yaml(&self) -> String {
        self.imported.to_yaml()
    }

    /// builtin + imported + current report.
    pub fn combined(&self) -> Knowledge {
        let mut k = self.builtin.clone();
        k.extend(self.imported.clone());
        k.extend(self.report.clone());
        k
    }
}

/// Map an assistant error to the UI error shape.
pub fn app_error(e: AssistantError) -> AppError {
    log::error!("assistant error: {e}");
    let code = match &e {
        AssistantError::ModelNotInstalled => "assistant_not_installed",
        AssistantError::Download(_) => "assistant_download",
        AssistantError::Checksum(_) => "assistant_checksum",
        AssistantError::Load(_) => "assistant_load",
        AssistantError::Generation(_) => "assistant_generation",
        AssistantError::Convert(_) => "assistant_convert",
        AssistantError::NoBackend => "assistant_no_backend",
        AssistantError::Other(_) => "assistant_error",
    };
    AppError::new(code, e.to_string(), e.hint())
}

/// Load the model with the given knowledge (blocking: reads the model file).
pub fn load_assistant(
    app_data: &std::path::Path,
    knowledge: Knowledge,
) -> Result<Assistant, AppError> {
    Assistant::load(app_data, knowledge).map_err(app_error)
}

/// Turn the UI's path strings into `PathBuf`s, dropping blanks.
pub fn to_paths(paths: &[String]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use verimodel_assistant::{Chunk, Task};

    fn chunk(source: &str, title: &str) -> Chunk {
        Chunk {
            source: source.into(),
            title: title.into(),
            text: "t".into(),
            keywords: vec![],
        }
    }

    fn kn(chunks: Vec<Chunk>) -> Knowledge {
        Knowledge { chunks }
    }

    fn report_named(name: &str) -> RunReport {
        let mut r = RunReport::default();
        r.model.file_name = name.into();
        r
    }

    #[test]
    fn rebuild_keeps_builtin_and_imported_and_replaces_the_report() {
        let mut parts = KnowledgeParts::new(kn(vec![chunk("docs/RULES.md", "Rules")]));
        assert_eq!(parts.combined().chunks.len(), 1);

        assert_eq!(
            parts.add_imported(kn(vec![
                chunk("bep.docx", "Naming"),
                chunk("bep.docx", "Psets")
            ])),
            2
        );
        assert_eq!(parts.combined().chunks.len(), 3);

        parts.set_report(&report_named("a.ifc"));
        let first = parts.combined();
        assert!(first.chunks.iter().any(|c| c.source == "report:a.ifc"));
        let with_a = first.chunks.len();

        parts.set_report(&report_named("b.ifc"));
        let second = parts.combined();
        assert!(
            !second.chunks.iter().any(|c| c.source == "report:a.ifc"),
            "previous report chunks are gone"
        );
        assert!(second.chunks.iter().any(|c| c.source == "report:b.ifc"));
        assert_eq!(
            second.chunks.len(),
            with_a,
            "same shape of report, same chunk count"
        );
        assert!(second.chunks.iter().any(|c| c.source == "docs/RULES.md"));
        assert_eq!(
            second
                .chunks
                .iter()
                .filter(|c| c.source == "bep.docx")
                .count(),
            2
        );
    }

    #[test]
    fn imported_yaml_round_trips_and_excludes_builtin_and_report() {
        let mut parts = KnowledgeParts::new(kn(vec![chunk("docs/RULES.md", "Rules")]));
        parts.add_imported(kn(vec![chunk("bep.md", "Naming")]));
        parts.set_report(&report_named("a.ifc"));
        let back = Knowledge::from_yaml(&parts.imported_yaml()).unwrap();
        assert_eq!(back.chunks.len(), 1);
        assert_eq!(back.chunks[0].source, "bep.md");
    }

    #[test]
    fn model_not_installed_maps_to_code_and_hint() {
        let e = app_error(AssistantError::ModelNotInstalled);
        assert_eq!(e.code, "assistant_not_installed");
        assert!(e.message.contains("not installed"));
        assert!(e.hint.contains("Download model"));
    }

    #[test]
    fn app_info_reports_name_and_version() {
        let i = app_info();
        assert_eq!(i.name, "VeriModel");
        assert!(!i.version.is_empty());
    }

    #[test]
    fn task_json_round_trips_with_the_shapes_the_ui_sends() {
        let filter = serde_json::json!({
            "kind": "filter",
            "question": "errors on level 1",
            "context": { "rule_ids": ["CL-001"], "storeys": ["Level 1"], "ifc_types": ["IfcWall"], "severities": ["ERROR", "WARNING"] }
        });
        let t: Task = serde_json::from_value(filter.clone()).unwrap();
        assert!(matches!(t, Task::Filter { .. }));
        assert_eq!(serde_json::to_value(&t).unwrap(), filter);

        let explain = serde_json::json!({
            "kind": "explain",
            "finding": {
                "rule_id": "ID-002", "severity": "ERROR", "global_id": "1x2y3z", "step_id": 776, "ifc_type": "IfcDoor",
                "name": "DR-104", "storey": "Level 1", "message": "bad id", "expected": "22 chars", "actual": "x",
                "fix_hint": "re-export", "source": { "kind": "rule_pack", "pack": "generic" }
            },
            "rule": null
        });
        let t: Task = serde_json::from_value(explain).unwrap();
        match &t {
            Task::Explain { finding, rule } => {
                assert_eq!(finding.rule_id, "ID-002");
                assert!(rule.is_none());
            }
            other => panic!("wrong task {other:?}"),
        }
        let again: Task = serde_json::from_value(serde_json::to_value(&t).unwrap()).unwrap();
        assert_eq!(again, t);

        for v in [
            serde_json::json!({ "kind": "config_help", "question": "q", "current_yaml": null }),
            serde_json::json!({ "kind": "draft_rules", "requirements_text": "walls need FireRating" }),
            serde_json::json!({ "kind": "ask", "question": "what is IDS?" }),
        ] {
            let t: Task = serde_json::from_value(v.clone()).unwrap();
            assert_eq!(serde_json::to_value(&t).unwrap(), v);
        }
    }

    #[test]
    fn status_serialises_flat_with_loaded_flag() {
        let s = AssistantStatus {
            status: verimodel_assistant::model::status(std::path::Path::new("/nonexistent")),
            loaded: false,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["installed"], false);
        assert_eq!(v["loaded"], false);
        assert!(v["name"].is_string() && v["license"].is_string());
    }

    #[test]
    fn blank_paths_are_dropped() {
        assert_eq!(
            to_paths(&["a.md".into(), "  ".into()]),
            vec![PathBuf::from("a.md")]
        );
    }
}
