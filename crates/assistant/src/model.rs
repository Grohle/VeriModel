//! Model catalogue, on-disk layout, status and download.
//!
//! Models are NOT bundled with the installer. The user downloads one from the
//! Assistant panel (an explicit action, shown with size and license); it is
//! stored under `<app data>/assistant/<model id>/` and verified by SHA-256.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AssistantResult;

/// A downloadable model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpec {
    /// Folder name, e.g. "qwen2.5-0.5b-instruct-q4_k_m".
    pub id: String,
    pub name: String,
    /// "qwen2" | "llama" | "smollm" — selects the Candle model family and the chat template.
    pub family: String,
    pub gguf_url: String,
    pub gguf_sha256: String,
    pub gguf_size_bytes: u64,
    pub tokenizer_url: String,
    pub tokenizer_sha256: String,
    pub license: String,
    pub approx_ram_mb: u32,
}

/// Default model: Qwen2.5-0.5B-Instruct, Q4_K_M (~470 MB, Apache-2.0).
pub static DEFAULT_MODEL: std::sync::LazyLock<ModelSpec> = std::sync::LazyLock::new(|| ModelSpec {
    id: "qwen2.5-0.5b-instruct-q4_k_m".into(),
    name: "Qwen2.5 0.5B Instruct (Q4_K_M)".into(),
    family: "qwen2".into(),
    gguf_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf".into(),
    gguf_sha256: String::new(), // filled in by the implementer after verifying the download
    gguf_size_bytes: 491_400_032,
    tokenizer_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/resolve/main/tokenizer.json".into(),
    tokenizer_sha256: String::new(),
    license: "Apache-2.0".into(),
    approx_ram_mb: 700,
});

/// All models the UI may offer.
pub fn catalogue() -> Vec<ModelSpec> {
    vec![DEFAULT_MODEL.clone()]
}

/// Install state of a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelStatus {
    pub model_id: String,
    pub name: String,
    pub installed: bool,
    pub model_dir: PathBuf,
    pub gguf_path: PathBuf,
    pub tokenizer_path: PathBuf,
    pub gguf_size_bytes: u64,
    pub license: String,
    pub approx_ram_mb: u32,
    /// Which inference backend this build has ("candle" or "none").
    pub backend: String,
}

/// `<app data>/assistant/<model id>`.
pub fn model_dir(app_data: &Path, spec: &ModelSpec) -> PathBuf {
    app_data.join("assistant").join(&spec.id)
}

/// Status of the default model.
pub fn status(app_data: &Path) -> ModelStatus {
    status_of(app_data, &DEFAULT_MODEL)
}

pub fn status_of(app_data: &Path, spec: &ModelSpec) -> ModelStatus {
    let dir = model_dir(app_data, spec);
    let gguf = dir.join("model.gguf");
    let tok = dir.join("tokenizer.json");
    ModelStatus {
        model_id: spec.id.clone(),
        name: spec.name.clone(),
        installed: gguf.is_file() && tok.is_file(),
        model_dir: dir,
        gguf_path: gguf,
        tokenizer_path: tok,
        gguf_size_bytes: spec.gguf_size_bytes,
        license: spec.license.clone(),
        approx_ram_mb: spec.approx_ram_mb,
        backend: if cfg!(feature = "candle") { "candle".into() } else { "none".into() },
    }
}

/// Download progress callback: (bytes_done, bytes_total_if_known).
pub type Progress<'a> = &'a mut dyn FnMut(u64, Option<u64>);

/// Download `spec` into `app_data`, verify SHA-256 (when the spec declares one),
/// and return the status. STUB: to be implemented with `ureq` streaming to a
/// `.part` file renamed on success; never leaves a half file as installed.
pub fn download(app_data: &Path, spec: &ModelSpec, progress: Progress<'_>) -> AssistantResult<ModelStatus> {
    let _ = (app_data, spec, progress);
    Err(crate::error::AssistantError::Download("not implemented yet".into()))
}

/// Remove an installed model.
pub fn remove(app_data: &Path, spec: &ModelSpec) -> AssistantResult<()> {
    let dir = model_dir(app_data, spec);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}
