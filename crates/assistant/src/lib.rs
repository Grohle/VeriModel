//! # VeriModel assistant
//!
//! A small local language model (SLM) used as a **copilot for navigation,
//! explanation and configuration help**. It is read-only by construction:
//!
//! * It never takes part in a verdict. Pass/fail, scores, quantities and BCF
//!   issues are computed by `verimodel-core` before the assistant exists.
//! * Its only input is text: the user's question, a few retrieved knowledge
//!   chunks (built-in docs, imported documentation, the current report) and a
//!   task-specific JSON schema. It has no tools, no file-system access and no
//!   network access; the model download is a separate, explicit user action.
//! * Its output is parsed and validated against the task's schema and against
//!   the current report (a filter may only name rule ids, storeys and types
//!   that exist). Anything that does not validate is returned as plain text
//!   marked as such, never applied silently.
//! * Generation is greedy (temperature 0), so the same question with the same
//!   context gives the same answer.
//!
//! Module map: [`model`] (catalogue, download, status), [`backend`] (inference
//! trait + Candle implementation + a mock for tests), [`knowledge`] (BM25
//! retrieval over YAML knowledge), [`tasks`] (prompts, schemas, validation),
//! [`convert`] (documentation → knowledge YAML).

pub mod backend;
pub mod convert;
pub mod error;
pub mod knowledge;
pub mod model;
pub mod tasks;

pub use backend::{Backend, GenOptions, MockBackend};
pub use error::{AssistantError, AssistantResult};
pub use knowledge::{Chunk, Knowledge};
pub use model::{ModelSpec, ModelStatus, DEFAULT_MODEL};
pub use tasks::{Answer, FilterSpec, ReportContext, Task};

/// The assistant: one backend + one knowledge base.
pub struct Assistant {
    backend: Box<dyn Backend>,
    knowledge: Knowledge,
}

impl Assistant {
    /// Build with any backend (tests use [`MockBackend`]).
    pub fn with_backend(backend: Box<dyn Backend>, knowledge: Knowledge) -> Assistant {
        Assistant { backend, knowledge }
    }

    /// Load the installed model from `app_data` (see [`model::model_dir`]) with the Candle backend.
    /// Returns `AssistantError::ModelNotInstalled` when no model is present.
    pub fn load(app_data: &std::path::Path, knowledge: Knowledge) -> AssistantResult<Assistant> {
        let backend = backend::load_default(app_data)?;
        Ok(Assistant { backend, knowledge })
    }

    pub fn knowledge(&self) -> &Knowledge {
        &self.knowledge
    }

    pub fn knowledge_mut(&mut self) -> &mut Knowledge {
        &mut self.knowledge
    }

    pub fn backend_name(&self) -> &str {
        self.backend.name()
    }

    /// Run one task: retrieve context, prompt, generate, parse, validate.
    pub fn run(&mut self, task: &Task) -> AssistantResult<Answer> {
        tasks::run(self.backend.as_mut(), &self.knowledge, task)
    }
}
