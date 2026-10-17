//! Inference backends. The assistant only ever calls [`Backend::generate`]
//! with a prompt string; a backend has no access to anything else.

use std::path::Path;

use crate::error::{AssistantError, AssistantResult};

#[cfg(feature = "candle")]
pub mod candle;
pub mod mock;

pub use mock::MockBackend;

/// Generation settings. Greedy by default so answers are reproducible.
#[derive(Debug, Clone)]
pub struct GenOptions {
    pub max_tokens: usize,
    /// 0.0 = greedy (default). Anything else samples with this temperature.
    pub temperature: f64,
    /// Generation stops when any of these strings appears.
    pub stop: Vec<String>,
}

impl Default for GenOptions {
    fn default() -> Self {
        Self { max_tokens: 384, temperature: 0.0, stop: vec!["<|im_end|>".into(), "<|endoftext|>".into()] }
    }
}

/// A chat message for prompt building.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

/// Text in, text out. Nothing else.
pub trait Backend: Send {
    fn name(&self) -> &str;
    /// Generate a completion for a chat-formatted prompt (see [`chatml`]).
    fn generate(&mut self, prompt: &str, opts: &GenOptions) -> AssistantResult<String>;
}

/// ChatML prompt (Qwen2.5 / SmolLM2 format). Llama 3.x uses a different
/// template; the Candle backend maps by model family.
pub fn chatml(messages: &[Message]) -> String {
    let mut s = String::new();
    for m in messages {
        s.push_str("<|im_start|>");
        s.push_str(m.role);
        s.push('\n');
        s.push_str(&m.content);
        s.push_str("<|im_end|>\n");
    }
    s.push_str("<|im_start|>assistant\n");
    s
}

/// Load the default installed model with the default backend.
pub fn load_default(app_data: &Path) -> AssistantResult<Box<dyn Backend>> {
    #[cfg(feature = "candle")]
    {
        let status = crate::model::status(app_data);
        if !status.installed {
            return Err(AssistantError::ModelNotInstalled);
        }
        let b = candle::CandleBackend::load(&status.gguf_path, &status.tokenizer_path)?;
        Ok(Box::new(b))
    }
    #[cfg(not(feature = "candle"))]
    {
        let _ = app_data;
        Err(AssistantError::NoBackend)
    }
}
