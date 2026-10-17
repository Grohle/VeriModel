//! VeriModel desktop shell (Tauri v2).
//!
//! The window is a static HTML UI (`../frontend`) that talks to this crate through
//! the commands below. Every command is a thin wrapper around a plain function in
//! [`ops`], which is unit-tested without launching Tauri.
//!
//! Commands (argument names are snake_case on the JavaScript side too, e.g.
//! `invoke("run_check", { ifc_path, ids_path, config_yaml, out_dir })`):
//!
//! | command | returns |
//! |---|---|
//! | `inspect_file(path)` | `FileCard` |
//! | `run_check(ifc_path, ids_path?, config_yaml?, out_dir?, config_dir?)` | `RunOutcome` |
//! | `get_element(global_id)` | `Element` of the last run |
//! | `list_runs(root?)` | `RunLedger[]` |
//! | `default_config_yaml()` | `string` |
//! | `validate_config_yaml(yaml)` | `{ ok, error }` |
//! | `list_examples()` | `Example[]` |
//! | `open_path(path)` | `void` |
//! | `save_text_file(path, contents)` | `void` |
//! | `read_text_file(path)` | `string` (a project.yaml; remembers its folder for relative paths) |
//! | `app_info()` | `{ name, version }` |
//! | `assistant_status()` | `ModelStatus` + `{ loaded }` |
//! | `assistant_download_model()` | `ModelStatus` (emits `assistant://download-progress` `{ done, total }`) |
//! | `assistant_remove_model()` | `ModelStatus` |
//! | `assistant_ask(task)` | `Answer` (task is the JSON of `verimodel_assistant::Task`, tagged `kind`) |
//! | `assistant_import_docs(paths)` | `{ files_read, skipped, chunks_added, knowledge_yaml }` |
//! | `assistant_knowledge_yaml()` | `string` (imported documentation as knowledge.yaml) |
//!
//! The assistant is a read-only copilot: it never takes part in a verdict, score or quantity.
//!
//! Errors are `{ code, message, hint }` and never contain Debug output.

pub mod assistant;
pub mod error;
pub mod logging;
pub mod ops;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use verimodel_assistant::model::{self as assistant_model, DEFAULT_MODEL};
use verimodel_assistant::{Answer, Assistant, Knowledge, Task};
use verimodel_core::model::{Element, Model, RunLedger};
use verimodel_core::run::RunOutcome;

use crate::assistant::{
    AppInfo, AssistantStatus, ImportSummary, KnowledgeParts, DOWNLOAD_PROGRESS_EVENT,
};
use crate::error::AppError;
use crate::ops::{Example, ExampleRoots, FileCard, RunRequest, ValidationResult};

/// Managed state: the model of the last run, for the element detail panel.
#[derive(Default)]
pub struct AppState {
    last_model: Arc<Mutex<Option<Model>>>,
    /// Folder of the last text file read through `read_text_file` (a project.yaml). The UI
    /// sends the config as text, so this is where its relative paths (rates CSV, IDS, rule
    /// pack) resolve from.
    last_config_dir: Mutex<Option<PathBuf>>,
}

/// Managed state of the assistant: the loaded model (lazy) and what it can retrieve.
pub struct AssistantState {
    /// `None` until the first question; reset when the model is downloaded or removed.
    assistant: Mutex<Option<Assistant>>,
    /// Built-in docs + imported documentation + the current report, kept apart.
    knowledge: Mutex<KnowledgeParts>,
}

impl Default for AssistantState {
    fn default() -> Self {
        Self {
            assistant: Mutex::new(None),
            knowledge: Mutex::new(KnowledgeParts::new(Knowledge::builtin())),
        }
    }
}

impl AssistantState {
    /// Push the rebuilt knowledge into the loaded assistant (if any).
    fn sync_loaded(&self, knowledge: Knowledge) {
        if let Ok(mut a) = self.assistant.lock() {
            if let Some(a) = a.as_mut() {
                *a.knowledge_mut() = knowledge;
            }
        }
    }
}

fn app_data(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path().app_data_dir().map_err(|e| {
        AppError::new(
            "no_data_dir",
            "VeriModel cannot find its data folder.",
            format!("Details were written to the log: {e}"),
        )
    })
}

fn runs_root(app: &AppHandle) -> Result<PathBuf, AppError> {
    let base = app.path().app_data_dir().map_err(|e| {
        AppError::new(
            "no_data_dir",
            "VeriModel cannot find its data folder.",
            format!("Details were written to the log: {e}"),
        )
    })?;
    Ok(base.join("runs"))
}

async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(format!("background task failed: {e}")))?
}

#[tauri::command(rename_all = "snake_case")]
async fn inspect_file(path: String) -> Result<FileCard, AppError> {
    blocking(move || ops::inspect_file(&path)).await
}

#[tauri::command(rename_all = "snake_case")]
async fn run_check(
    app: AppHandle,
    state: State<'_, AppState>,
    ifc_path: String,
    ids_path: Option<String>,
    config_yaml: Option<String>,
    out_dir: Option<String>,
    config_dir: Option<String>,
) -> Result<RunOutcome, AppError> {
    let ifc = PathBuf::from(&ifc_path);
    let out = match out_dir.filter(|s| !s.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => ops::default_out_dir(&runs_root(&app)?, &ifc, &ops::timestamp_now()),
    };
    let req = RunRequest {
        ifc_path: ifc,
        ids_path: ids_path.filter(|s| !s.trim().is_empty()).map(PathBuf::from),
        config_yaml,
        config_dir: config_dir
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .or_else(|| state.last_config_dir.lock().ok().and_then(|g| g.clone())),
        out_dir: out,
    };
    let slot = Arc::clone(&state.last_model);
    let handle = app.clone();
    blocking(move || {
        let (outcome, model) = ops::run_check_blocking(&req)?;
        if let Ok(mut guard) = slot.lock() {
            *guard = Some(model);
        }
        // The assistant can now answer questions about this report (replacing the previous one).
        let assist = handle.state::<AssistantState>();
        let combined = assist.knowledge.lock().ok().map(|mut k| {
            k.set_report(&outcome.report);
            k.combined()
        });
        if let Some(k) = combined {
            assist.sync_loaded(k);
        }
        Ok(outcome)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
fn get_element(state: State<'_, AppState>, global_id: String) -> Result<Element, AppError> {
    let guard = state
        .last_model
        .lock()
        .map_err(|_| AppError::internal("model lock poisoned"))?;
    let model = guard.as_ref().ok_or_else(|| {
        AppError::new(
            "no_model",
            "No model has been checked yet.",
            "Run a check first, then select an element.",
        )
    })?;
    model.element_by_id(&global_id).cloned().ok_or_else(|| {
        AppError::new(
            "element_not_found",
            format!("Element {global_id} is not in the last checked model."),
            "Run the check again.",
        )
    })
}

#[tauri::command(rename_all = "snake_case")]
async fn list_runs(app: AppHandle, root: Option<String>) -> Result<Vec<RunLedger>, AppError> {
    let root = match root.filter(|s| !s.trim().is_empty()) {
        Some(r) => PathBuf::from(r),
        None => runs_root(&app)?,
    };
    blocking(move || Ok(verimodel_core::run::list_runs(&root)?)).await
}

#[tauri::command(rename_all = "snake_case")]
fn default_config_yaml() -> String {
    ops::default_config_yaml()
}

#[tauri::command(rename_all = "snake_case")]
fn validate_config_yaml(yaml: String) -> ValidationResult {
    ops::validate_config_yaml(&yaml)
}

/// Resource folders of the installed app, if the bundle has them.
fn bundled_roots(app: &AppHandle) -> Option<ExampleRoots> {
    let examples = app
        .path()
        .resolve("examples", BaseDirectory::Resource)
        .ok()?;
    let fixtures = app
        .path()
        .resolve("fixtures", BaseDirectory::Resource)
        .ok()?;
    Some(ExampleRoots { examples, fixtures })
}

#[tauri::command(rename_all = "snake_case")]
fn list_examples(app: AppHandle) -> Vec<Example> {
    let dev_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let roots = ops::resolve_roots(bundled_roots(&app), dev_root);
    ops::list_examples_in(&roots)
}

#[tauri::command(rename_all = "snake_case")]
fn open_path(app: AppHandle, path: String) -> Result<(), AppError> {
    if !Path::new(&path).exists() {
        return Err(AppError::file_missing("File or folder", &path));
    }
    app.opener().open_path(&path, None::<&str>).map_err(|e| {
        log::error!("open_path failed for {path}: {e:?}");
        AppError::new(
            "open_failed",
            format!("Could not open {path}."),
            "Open it from your file manager instead.",
        )
    })
}

#[tauri::command(rename_all = "snake_case")]
fn save_text_file(path: String, contents: String) -> Result<(), AppError> {
    ops::save_text_file(&path, &contents)
}

#[tauri::command(rename_all = "snake_case")]
fn read_text_file(state: State<'_, AppState>, path: String) -> Result<String, AppError> {
    let text = ops::read_text_file(&path)?;
    if let (Some(dir), Ok(mut slot)) = (Path::new(&path).parent(), state.last_config_dir.lock()) {
        *slot = Some(dir.to_path_buf());
    }
    Ok(text)
}

// ---------------------------------------------------------------------------------------------
// Assistant (optional, local, read-only)
// ---------------------------------------------------------------------------------------------

#[tauri::command(rename_all = "snake_case")]
fn app_info() -> AppInfo {
    assistant::app_info()
}

fn status_now(app: &AppHandle, state: &AssistantState) -> Result<AssistantStatus, AppError> {
    let loaded = state.assistant.lock().map(|a| a.is_some()).unwrap_or(false);
    Ok(AssistantStatus {
        status: assistant_model::status(&app_data(app)?),
        loaded,
    })
}

#[tauri::command(rename_all = "snake_case")]
fn assistant_status(
    app: AppHandle,
    state: State<'_, AssistantState>,
) -> Result<AssistantStatus, AppError> {
    status_now(&app, &state)
}

#[tauri::command(rename_all = "snake_case")]
async fn assistant_download_model(app: AppHandle) -> Result<AssistantStatus, AppError> {
    let data = app_data(&app)?;
    let handle = app.clone();
    blocking(move || {
        let spec = DEFAULT_MODEL.clone();
        let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut last_done = 0u64;
        let mut progress = |done: u64, total: Option<u64>| {
            let total = total.unwrap_or(spec.gguf_size_bytes);
            // About 5 events a second at most; always emit the last one.
            if done < total
                && last_emit.elapsed().as_millis() < 200
                && done.saturating_sub(last_done) < total / 100
            {
                return;
            }
            last_emit = std::time::Instant::now();
            last_done = done;
            let _ = handle.emit(
                DOWNLOAD_PROGRESS_EVENT,
                serde_json::json!({ "done": done, "total": total }),
            );
        };
        let status =
            assistant_model::download(&data, &spec, &mut progress).map_err(assistant::app_error)?;
        let state = handle.state::<AssistantState>();
        if let Ok(mut a) = state.assistant.lock() {
            *a = None; // reload the fresh files on the next question
        }
        Ok(AssistantStatus {
            status,
            loaded: false,
        })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
async fn assistant_remove_model(app: AppHandle) -> Result<AssistantStatus, AppError> {
    let data = app_data(&app)?;
    let handle = app.clone();
    blocking(move || {
        let state = handle.state::<AssistantState>();
        if let Ok(mut a) = state.assistant.lock() {
            *a = None; // release the model before deleting its files
        }
        assistant_model::remove(&data, &DEFAULT_MODEL).map_err(assistant::app_error)?;
        Ok(AssistantStatus {
            status: assistant_model::status(&data),
            loaded: false,
        })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
async fn assistant_ask(app: AppHandle, task: Task) -> Result<Answer, AppError> {
    let data = app_data(&app)?;
    let handle = app.clone();
    blocking(move || {
        let state = handle.state::<AssistantState>();
        let mut slot = state
            .assistant
            .lock()
            .map_err(|_| AppError::internal("assistant lock poisoned"))?;
        if slot.is_none() {
            let knowledge = state
                .knowledge
                .lock()
                .map_err(|_| AppError::internal("knowledge lock poisoned"))?
                .combined();
            *slot = Some(assistant::load_assistant(&data, knowledge)?);
        }
        let a = slot
            .as_mut()
            .ok_or_else(|| AppError::internal("assistant missing after load"))?;
        a.run(&task).map_err(assistant::app_error)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
async fn assistant_import_docs(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<ImportSummary, AppError> {
    let paths = assistant::to_paths(&paths);
    let handle = app.clone();
    blocking(move || {
        let report =
            verimodel_assistant::convert::convert_paths(&paths).map_err(assistant::app_error)?;
        let knowledge_yaml = report.knowledge.to_yaml();
        let state = handle.state::<AssistantState>();
        let (chunks_added, combined) = {
            let mut parts = state
                .knowledge
                .lock()
                .map_err(|_| AppError::internal("knowledge lock poisoned"))?;
            let n = parts.add_imported(report.knowledge);
            (n, parts.combined())
        };
        state.sync_loaded(combined);
        Ok(ImportSummary {
            files_read: report.files_read,
            skipped: report.skipped,
            chunks_added,
            knowledge_yaml,
        })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
fn assistant_knowledge_yaml(state: State<'_, AssistantState>) -> Result<String, AppError> {
    let parts = state
        .knowledge
        .lock()
        .map_err(|_| AppError::internal("knowledge lock poisoned"))?;
    Ok(parts.imported_yaml())
}

/// Start the app.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .manage(AssistantState::default())
        .setup(|app| {
            if let Ok(dir) = app.path().app_log_dir() {
                if let Some(p) = logging::init(&dir) {
                    log::info!(
                        "{} {} started, log file {}",
                        verimodel_core::TOOL_NAME,
                        verimodel_core::VERSION,
                        p.display()
                    );
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            inspect_file,
            run_check,
            get_element,
            list_runs,
            default_config_yaml,
            validate_config_yaml,
            list_examples,
            open_path,
            save_text_file,
            read_text_file,
            app_info,
            assistant_status,
            assistant_download_model,
            assistant_remove_model,
            assistant_ask,
            assistant_import_docs,
            assistant_knowledge_yaml,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VeriModel");
}
