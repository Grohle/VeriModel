//! Tasks: what the assistant can be asked, how it is prompted, and how its
//! answer is validated before anyone sees it.
//!
//! STUB: `run` to be implemented. Contract:
//! * Build messages: a system prompt stating the role and the JSON schema of the
//!   expected answer, then up to `k` retrieved chunks (BM25 over the knowledge),
//!   then the user content. Never include anything the user did not provide or
//!   that is not in the knowledge base.
//! * Generate with `GenOptions::default()` (greedy), parse the first JSON
//!   object in the output, deserialise into the task's answer type, validate
//!   (see each variant), and on failure retry ONCE with the parser error
//!   appended; if it still fails return `Answer::Text { text, structured: false }`.
//! * `Task::Filter`: answer `FilterSpec`; every `rule_id`, `storey`, `ifc_type`,
//!   `severity` must be in `ReportContext` (case-insensitive match → canonical
//!   value) else dropped and listed in `dropped`.
//! * `Task::Explain`: answer text (2–5 sentences, plain language, mentions the
//!   element name/GlobalId and the fix). Input facts are the finding + rule.
//! * `Task::ConfigHelp`: answer `{ yaml, explanation }`; `yaml` is validated
//!   with `verimodel_core::Config::from_yaml` → `valid`, `validation_error`.
//! * `Task::DraftRules`: requirement text (e.g. from converted documentation)
//!   → a `required_psets` / `naming` / `classification` YAML fragment, validated
//!   the same way.
//! * `Task::Ask`: free question → `Answer::Text` with the titles of the chunks
//!   used as `sources`.

use serde::{Deserialize, Serialize};

use verimodel_core::model::{Finding, RuleResult};

use crate::backend::Backend;
use crate::error::AssistantResult;
use crate::knowledge::Knowledge;

/// Allowed values for filter answers: taken from the current report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReportContext {
    pub rule_ids: Vec<String>,
    pub storeys: Vec<String>,
    pub ifc_types: Vec<String>,
    /// "ERROR" | "WARNING" | "INFO"
    pub severities: Vec<String>,
}

/// A UI filter the assistant proposes; the UI applies it only after the user confirms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilterSpec {
    #[serde(default)]
    pub severity: Vec<String>,
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub storey: Option<String>,
    #[serde(default)]
    pub ifc_type: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub explanation: String,
    /// Values the model proposed that do not exist in the report (dropped).
    #[serde(default)]
    pub dropped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Task {
    Filter { question: String, context: ReportContext },
    Explain { finding: Finding, rule: Option<RuleResult> },
    ConfigHelp { question: String, current_yaml: Option<String> },
    DraftRules { requirements_text: String },
    Ask { question: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Filter { filter: FilterSpec },
    Explanation { text: String },
    ConfigSnippet { yaml: String, valid: bool, validation_error: Option<String>, explanation: String },
    Text { text: String, sources: Vec<String>, structured: bool },
}

/// Number of retrieved chunks put in the prompt.
pub const RETRIEVE_K: usize = 4;

/// Run a task against a backend with the given knowledge.
pub fn run(backend: &mut dyn Backend, knowledge: &Knowledge, task: &Task) -> AssistantResult<Answer> {
    let _ = (backend, knowledge, task);
    Err(crate::error::AssistantError::Generation("tasks not implemented yet".into()))
}
