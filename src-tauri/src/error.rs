//! Error type sent to the UI: plain language only, never a Debug dump.
//! Technical detail goes to the log file instead.

use serde::Serialize;
use verimodel_core::CoreError;
use verimodel_ifc_reader::ReaderError;

/// What the frontend receives when a command fails.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AppError {
    /// Stable machine code, e.g. "ifc_not_found", "config_invalid".
    pub code: String,
    /// What went wrong, in plain language.
    pub message: String,
    /// What the user can do about it (may be empty).
    pub hint: String,
}

impl AppError {
    pub fn new(code: &str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            hint: hint.into(),
        }
    }

    pub fn file_missing(what: &str, path: &str) -> Self {
        Self::new(
            "file_not_found",
            format!("{what} not found: {path}"),
            "Check that the file still exists, or choose it again.",
        )
    }

    pub fn internal(detail: impl std::fmt::Display) -> Self {
        log::error!("internal error: {detail}");
        Self::new(
            "internal",
            "Something unexpected went wrong inside VeriModel.",
            "Try again. If it keeps happening, send the log file (Help > Open log folder) to the maintainers.",
        )
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<ReaderError> for AppError {
    fn from(e: ReaderError) -> Self {
        log::error!(
            "reader error [{}]: {} (hint: {})",
            e.code,
            e.message,
            e.hint
        );
        AppError {
            code: e.code,
            message: e.message,
            hint: e.hint,
        }
    }
}

impl From<CoreError> for AppError {
    fn from(e: CoreError) -> Self {
        log::error!("core error: {e:?}");
        let (code, hint) = match &e {
            CoreError::Config(_) => (
                "config_invalid",
                "Open the Project settings and fix the highlighted problem.",
            ),
            CoreError::RulePack(_) => (
                "rule_pack_invalid",
                "Check the custom rule pack referenced in the project settings.",
            ),
            CoreError::Io(_) => (
                "io_error",
                "Check that the file exists and that you can read and write that folder.",
            ),
            CoreError::Report(_) => (
                "report_error",
                "Choose another output folder and run the check again.",
            ),
            CoreError::Bcf(_) => (
                "bcf_error",
                "Choose another output folder and run the check again.",
            ),
            CoreError::Qto(_) => (
                "qto_error",
                "Check the unit rates file and the quantity takeoff settings.",
            ),
            CoreError::Other(_) => ("error", ""),
        };
        AppError::new(code, e.to_string(), hint)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        log::error!("io error: {e:?}");
        AppError::new(
            "io_error",
            format!("File problem: {e}"),
            "Check the path and your permissions.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_maps_to_code_and_plain_message() {
        let e: AppError = CoreError::Config("bad yaml".into()).into();
        assert_eq!(e.code, "config_invalid");
        assert!(e.message.contains("bad yaml"));
        assert!(
            !e.message.contains("Config("),
            "no Debug output in messages"
        );
        assert!(!e.hint.is_empty());
    }

    #[test]
    fn reader_error_keeps_code_message_hint() {
        let e: AppError = ReaderError {
            code: "python_missing".into(),
            message: "No Python".into(),
            hint: "Install it".into(),
        }
        .into();
        assert_eq!(
            (e.code.as_str(), e.message.as_str(), e.hint.as_str()),
            ("python_missing", "No Python", "Install it")
        );
    }

    #[test]
    fn serialises_to_three_fields() {
        let v = serde_json::to_value(AppError::new("c", "m", "h")).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"code": "c", "message": "m", "hint": "h"})
        );
    }
}
