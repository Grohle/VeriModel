//! Candle backend: quantized GGUF inference in pure Rust (CPU; Metal/CUDA via
//! candle features later). STUB: to be implemented.
//!
//! Contract:
//! * `CandleBackend::load(gguf, tokenizer_json)` reads the GGUF header to pick
//!   the architecture (`general.architecture`: "qwen2" | "llama" | ...) and
//!   builds the matching `candle_transformers::models::quantized_*` model.
//! * `generate` tokenises the ChatML prompt, runs greedy decoding (or sampling
//!   when `temperature > 0`) up to `max_tokens`, stops on EOS or any `stop`
//!   string, and returns the decoded text without the stop sequence.
//! * Must never panic on a bad file: return `AssistantError::Load`.

use std::path::Path;

use super::{Backend, GenOptions};
use crate::error::{AssistantError, AssistantResult};

pub struct CandleBackend {
    name: String,
}

impl CandleBackend {
    pub fn load(gguf: &Path, tokenizer: &Path) -> AssistantResult<CandleBackend> {
        let _ = (gguf, tokenizer);
        Err(AssistantError::Load(
            "candle backend not implemented yet".into(),
        ))
    }
}

impl Backend for CandleBackend {
    fn name(&self) -> &str {
        &self.name
    }
    fn generate(&mut self, _prompt: &str, _opts: &GenOptions) -> AssistantResult<String> {
        Err(AssistantError::Generation(
            "candle backend not implemented yet".into(),
        ))
    }
}
