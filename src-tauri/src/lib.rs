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
//!
//! Errors are `{ code, message, hint }` and never contain Debug output.

pub mod error;
pub mod logging;
pub mod ops;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;
use verimodel_core::model::{Element, Model, RunLedger};
use verimodel_core::run::RunOutcome;

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

fn runs_root(app: &AppHandle) -> Result<PathBuf, AppError> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::new("no_data_dir", "VeriModel cannot find its data folder.", format!("Details were written to the log: {e}")))?;
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
    blocking(move || {
        let (outcome, model) = ops::run_check_blocking(&req)?;
        if let Ok(mut guard) = slot.lock() {
            *guard = Some(model);
        }
        Ok(outcome)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
fn get_element(state: State<'_, AppState>, global_id: String) -> Result<Element, AppError> {
    let guard = state.last_model.lock().map_err(|_| AppError::internal("model lock poisoned"))?;
    let model = guard.as_ref().ok_or_else(|| {
        AppError::new("no_model", "No model has been checked yet.", "Run a check first, then select an element.")
    })?;
    model
        .element_by_id(&global_id)
        .cloned()
        .ok_or_else(|| AppError::new("element_not_found", format!("Element {global_id} is not in the last checked model."), "Run the check again."))
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
    let examples = app.path().resolve("examples", BaseDirectory::Resource).ok()?;
    let fixtures = app.path().resolve("fixtures", BaseDirectory::Resource).ok()?;
    Some(ExampleRoots { examples, fixtures })
}

#[tauri::command(rename_all = "snake_case")]
fn list_examples(app: AppHandle) -> Vec<Example> {
    let dev_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or_else(|| Path::new("."));
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
        AppError::new("open_failed", format!("Could not open {path}."), "Open it from your file manager instead.")
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

/// Start the app.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            if let Ok(dir) = app.path().app_log_dir() {
                if let Some(p) = logging::init(&dir) {
                    log::info!("{} {} started, log file {}", verimodel_core::TOOL_NAME, verimodel_core::VERSION, p.display());
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running VeriModel");
}
