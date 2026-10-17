//! Candle backend: quantized GGUF inference in pure Rust (CPU; Metal/CUDA via
//! candle features later).
//!
//! Contract:
//! * `CandleBackend::load(gguf, tokenizer_json)` reads the GGUF header to pick
//!   the architecture (`general.architecture`: "qwen2" | "llama") and builds
//!   the matching `candle_transformers::models::quantized_*` model.
//! * `generate` takes a ChatML prompt (see [`super::chatml`]); for the Llama
//!   family it is re-rendered with the Llama 3 chat template. It tokenises the
//!   prompt, runs greedy decoding (or sampling when `temperature > 0`) with a
//!   mild repeat penalty, up to `max_tokens`, stops on EOS or any `stop`
//!   string, and returns the decoded text without the stop sequence.
//! * The KV cache is cleared at the start of every call: each answer is
//!   computed from position 0, so the same prompt always gives the same text.
//! * Never panics on a bad file: returns `AssistantError::Load`.
//!
//! Performance: candle picks its AVX2/FMA quantized kernels at compile time
//! (`cfg(target_feature = "avx2")`). A default x86-64 build falls back to
//! scalar code and is several times slower; release builds should use
//! `-C target-cpu=x86-64-v3` (or `native` for local runs).

use std::path::Path;

use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::{quantized_llama, quantized_qwen2};
use tokenizers::Tokenizer;

use super::{Backend, GenOptions, Message};
use crate::error::{AssistantError, AssistantResult};

/// Repeat penalty applied to the last [`REPEAT_LAST_N`] generated tokens.
const REPEAT_PENALTY: f32 = 1.1;
const REPEAT_LAST_N: usize = 64;
/// Seed for sampling (only used when `temperature > 0`).
const SEED: u64 = 299_792_458;
/// Token strings treated as end-of-sequence when the tokenizer knows them.
const EOS_TOKENS: &[&str] = &[
    "<|im_end|>",
    "<|endoftext|>",
    "<|eot_id|>",
    "<|end_of_text|>",
];

enum Weights {
    Qwen2(quantized_qwen2::ModelWeights),
    Llama(quantized_llama::ModelWeights),
}

impl Weights {
    fn forward(&mut self, x: &Tensor, pos: usize) -> candle_core::Result<Tensor> {
        match self {
            Weights::Qwen2(m) => m.forward(x, pos),
            Weights::Llama(m) => m.forward(x, pos),
        }
    }

    fn clear_kv_cache(&mut self) {
        match self {
            Weights::Qwen2(m) => m.clear_kv_cache(),
            Weights::Llama(m) => m.clear_kv_cache(),
        }
    }
}

/// Timing of the last [`Backend::generate`] call (for diagnostics and benchmarks).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GenStats {
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    /// Time to process the prompt (first forward pass), in seconds.
    pub prefill_secs: f64,
    /// Time spent generating the remaining tokens, in seconds.
    pub decode_secs: f64,
}

impl GenStats {
    /// Prompt tokens processed per second.
    pub fn prefill_tokens_per_sec(&self) -> f64 {
        if self.prefill_secs > 0.0 {
            self.prompt_tokens as f64 / self.prefill_secs
        } else {
            0.0
        }
    }

    /// Generated tokens per second, excluding the prefill.
    pub fn decode_tokens_per_sec(&self) -> f64 {
        if self.decode_secs > 0.0 {
            self.generated_tokens.saturating_sub(1) as f64 / self.decode_secs
        } else {
            0.0
        }
    }
}

/// A loaded GGUF model plus its tokenizer.
pub struct CandleBackend {
    name: String,
    family: String,
    weights: Weights,
    tokenizer: Tokenizer,
    eos_ids: Vec<u32>,
    context_length: usize,
    device: Device,
    last_stats: Option<GenStats>,
}

impl CandleBackend {
    /// Load a GGUF model and its `tokenizer.json`. Never panics on a bad file.
    pub fn load(gguf: &Path, tokenizer: &Path) -> AssistantResult<CandleBackend> {
        let gguf = gguf.to_path_buf();
        let tokenizer = tokenizer.to_path_buf();
        match std::panic::catch_unwind(move || Self::load_inner(&gguf, &tokenizer)) {
            Ok(r) => r,
            Err(_) => Err(AssistantError::Load(
                "the model file is malformed (loader panicked)".into(),
            )),
        }
    }

    fn load_inner(gguf: &Path, tokenizer_path: &Path) -> AssistantResult<CandleBackend> {
        let mut file = std::fs::File::open(gguf)
            .map_err(|e| AssistantError::Load(format!("cannot open {}: {e}", gguf.display())))?;
        let content = gguf_file::Content::read(&mut file).map_err(|e| {
            AssistantError::Load(format!("{} is not a valid GGUF file: {e}", gguf.display()))
        })?;
        let arch = content
            .metadata
            .get("general.architecture")
            .and_then(|v| v.to_string().ok())
            .cloned()
            .ok_or_else(|| {
                AssistantError::Load("the GGUF file does not declare general.architecture".into())
            })?;
        let name = content
            .metadata
            .get("general.name")
            .and_then(|v| v.to_string().ok())
            .cloned()
            .unwrap_or_else(|| arch.clone());
        let context_length = content
            .metadata
            .get(&format!("{arch}.context_length"))
            .and_then(|v| v.to_u32().ok())
            .map(|v| v as usize)
            .unwrap_or(4096);
        let gguf_eos = content
            .metadata
            .get("tokenizer.ggml.eos_token_id")
            .and_then(|v| v.to_u32().ok());
        let device = Device::Cpu;
        let weights = match arch.as_str() {
            "qwen2" => Weights::Qwen2(
                quantized_qwen2::ModelWeights::from_gguf(content, &mut file, &device).map_err(
                    |e| AssistantError::Load(format!("cannot build the qwen2 model: {e}")),
                )?,
            ),
            "llama" => Weights::Llama(
                quantized_llama::ModelWeights::from_gguf(content, &mut file, &device).map_err(
                    |e| AssistantError::Load(format!("cannot build the llama model: {e}")),
                )?,
            ),
            other => {
                return Err(AssistantError::Load(format!(
                    "unsupported model architecture '{other}' (supported: qwen2, llama)"
                )))
            }
        };
        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| {
            AssistantError::Load(format!(
                "cannot read tokenizer {}: {e}",
                tokenizer_path.display()
            ))
        })?;
        let mut eos_ids: Vec<u32> = EOS_TOKENS
            .iter()
            .filter_map(|t| tokenizer.token_to_id(t))
            .collect();
        if let Some(id) = gguf_eos {
            if !eos_ids.contains(&id) {
                eos_ids.push(id);
            }
        }
        Ok(CandleBackend {
            name: format!("candle:{name}"),
            family: arch,
            weights,
            tokenizer,
            eos_ids,
            context_length,
            device,
            last_stats: None,
        })
    }

    /// Model family from the GGUF header ("qwen2" | "llama").
    pub fn family(&self) -> &str {
        &self.family
    }

    /// Token counts and timings of the last generation.
    pub fn last_stats(&self) -> Option<GenStats> {
        self.last_stats
    }

    /// Render the ChatML prompt in the template this model family expects.
    fn render_prompt(&self, prompt: &str) -> String {
        if self.family == "llama" {
            if let Some(messages) = parse_chatml(prompt) {
                // Keep any text already written in the open assistant turn (a primed answer prefix).
                let primed = prompt
                    .rsplit_once("<|im_start|>assistant\n")
                    .map(|(_, t)| t)
                    .filter(|t| !t.contains("<|im_end|>"))
                    .unwrap_or("");
                return format!("{}{primed}", llama3_chat(&messages));
            }
        }
        prompt.to_string()
    }

    fn generate_inner(&mut self, prompt: &str, opts: &GenOptions) -> AssistantResult<String> {
        let gen_err = |e: candle_core::Error| AssistantError::Generation(e.to_string());
        let prompt = self.render_prompt(prompt);
        let enc = self
            .tokenizer
            .encode(prompt.as_str(), false)
            .map_err(|e| AssistantError::Generation(format!("tokenizer: {e}")))?;
        let prompt_ids: Vec<u32> = enc.get_ids().to_vec();
        if prompt_ids.is_empty() {
            return Ok(String::new());
        }
        if prompt_ids.len() + opts.max_tokens > self.context_length {
            return Err(AssistantError::Generation(format!(
                "the prompt is too long ({} tokens, model context {})",
                prompt_ids.len(),
                self.context_length
            )));
        }
        let mut stops: Vec<String> = opts.stop.clone();
        if self.family == "llama" {
            stops.push("<|eot_id|>".into());
        }
        let mut sampler = if opts.temperature > 0.0 {
            Some(LogitsProcessor::new(SEED, Some(opts.temperature), None))
        } else {
            None
        };

        self.weights.clear_kv_cache();
        self.last_stats = None;
        let mut stats = GenStats {
            prompt_tokens: prompt_ids.len(),
            ..GenStats::default()
        };
        let started = std::time::Instant::now();
        let mut generated: Vec<u32> = Vec::new();
        let mut text = String::new();
        let mut pos = 0usize;
        let mut input = prompt_ids;
        while generated.len() < opts.max_tokens {
            let x = Tensor::new(input.as_slice(), &self.device)
                .and_then(|t| t.unsqueeze(0))
                .map_err(gen_err)?;
            let logits = self.weights.forward(&x, pos).map_err(gen_err)?;
            if pos == 0 {
                stats.prefill_secs = started.elapsed().as_secs_f64();
            }
            pos += input.len();
            let logits = logits
                .squeeze(0)
                .and_then(|l| l.to_dtype(candle_core::DType::F32))
                .map_err(gen_err)?;
            let logits = if generated.is_empty() {
                logits
            } else {
                let start = generated.len().saturating_sub(REPEAT_LAST_N);
                candle_transformers::utils::apply_repeat_penalty(
                    &logits,
                    REPEAT_PENALTY,
                    &generated[start..],
                )
                .map_err(gen_err)?
            };
            let next = match sampler.as_mut() {
                Some(s) => s.sample(&logits).map_err(gen_err)?,
                None => logits
                    .argmax(candle_core::D::Minus1)
                    .and_then(|t| t.to_scalar::<u32>())
                    .map_err(gen_err)?,
            };
            stats.generated_tokens += 1;
            stats.decode_secs = started.elapsed().as_secs_f64() - stats.prefill_secs;
            self.last_stats = Some(stats);
            if self.eos_ids.contains(&next) {
                break;
            }
            generated.push(next);
            text = self
                .tokenizer
                .decode(&generated, false)
                .map_err(|e| AssistantError::Generation(format!("tokenizer: {e}")))?;
            if let Some(cut) = first_stop(&text, &stops) {
                text.truncate(cut);
                return Ok(text);
            }
            input = vec![next];
        }
        Ok(text)
    }
}

/// Byte offset of the earliest stop string in `text`, if any.
fn first_stop(text: &str, stops: &[String]) -> Option<usize> {
    stops
        .iter()
        .filter(|s| !s.is_empty())
        .filter_map(|s| text.find(s.as_str()))
        .min()
}

impl Backend for CandleBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn generate(&mut self, prompt: &str, opts: &GenOptions) -> AssistantResult<String> {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.generate_inner(prompt, opts)
        }));
        match r {
            Ok(r) => r,
            Err(_) => {
                self.weights.clear_kv_cache();
                Err(AssistantError::Generation(
                    "inference failed unexpectedly".into(),
                ))
            }
        }
    }
}

/// Parse a prompt produced by [`super::chatml`] back into messages. The final
/// open `assistant` turn is dropped. Returns `None` if the text is not ChatML.
pub fn parse_chatml(prompt: &str) -> Option<Vec<Message>> {
    let mut out = Vec::new();
    let mut rest = prompt;
    while let Some(start) = rest.find("<|im_start|>") {
        rest = &rest[start + "<|im_start|>".len()..];
        let nl = rest.find('\n')?;
        let role = match rest[..nl].trim() {
            "system" => "system",
            "user" => "user",
            "assistant" => "assistant",
            _ => return None,
        };
        rest = &rest[nl + 1..];
        match rest.find("<|im_end|>") {
            Some(end) => {
                out.push(Message {
                    role,
                    content: rest[..end].to_string(),
                });
                rest = &rest[end + "<|im_end|>".len()..];
            }
            None => break, // the open assistant turn
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Llama 3.x chat template, ending with an open assistant header.
pub fn llama3_chat(messages: &[Message]) -> String {
    let mut s = String::from("<|begin_of_text|>");
    for m in messages {
        s.push_str("<|start_header_id|>");
        s.push_str(m.role);
        s.push_str("<|end_header_id|>\n\n");
        s.push_str(m.content.trim());
        s.push_str("<|eot_id|>");
    }
    s.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatml_roundtrip_to_llama3() {
        let msgs = vec![
            Message {
                role: "system",
                content: "sys".into(),
            },
            Message {
                role: "user",
                content: "hi\nthere".into(),
            },
        ];
        let p = super::super::chatml(&msgs);
        let parsed = parse_chatml(&p).unwrap();
        assert_eq!(parsed, msgs);
        let l = llama3_chat(&parsed);
        assert_eq!(
            l,
            "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\nsys<|eot_id|><|start_header_id|>user<|end_header_id|>\n\nhi\nthere<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n"
        );
        assert!(parse_chatml("plain text").is_none());
    }

    #[test]
    fn bad_files_are_load_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path().join("model.gguf");
        let t = dir.path().join("tokenizer.json");
        std::fs::write(&g, b"GGUF garbage").unwrap();
        std::fs::write(&t, b"{}").unwrap();
        assert!(matches!(
            CandleBackend::load(&g, &t),
            Err(AssistantError::Load(_))
        ));
        let missing = dir.path().join("nope.gguf");
        assert!(matches!(
            CandleBackend::load(&missing, &t),
            Err(AssistantError::Load(_))
        ));
    }

    #[test]
    fn stop_strings() {
        assert_eq!(
            first_stop("abc<|im_end|>x", &["<|im_end|>".into()]),
            Some(3)
        );
        assert_eq!(
            first_stop("abc", &["<|im_end|>".into(), String::new()]),
            None
        );
    }
}
