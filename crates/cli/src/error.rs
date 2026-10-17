//! Human-readable CLI errors: `Error: ...` and `Hint: ...`, never a Debug dump.

use std::fmt;

use verimodel_core::CoreError;
use verimodel_ifc_reader::ReaderError;

/// Exit code for a failed gate (verdict FAIL).
pub const EXIT_FAIL: i32 = 1;
/// Exit code for bad input or an operational error.
pub const EXIT_ERROR: i32 = 2;
/// Exit code for a feature that is planned but not implemented yet (clash).
pub const EXIT_NOT_IMPLEMENTED: i32 = 3;

/// An error the user can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub message: String,
    pub hint: Option<String>,
}

impl CliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), hint: None }
    }

    pub fn with_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        let hint = hint.into();
        Self { message: message.into(), hint: if hint.trim().is_empty() { None } else { Some(hint) } }
    }

    /// Text printed to stderr.
    pub fn render(&self) -> String {
        match &self.hint {
            Some(h) => format!("Error: {}\nHint: {}", self.message, h),
            None => format!("Error: {}", self.message),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<ReaderError> for CliError {
    fn from(e: ReaderError) -> Self {
        log::debug!("reader error [{}]: {}", e.code, e.message);
        CliError::with_hint(e.message, e.hint)
    }
}

impl From<CoreError> for CliError {
    fn from(e: CoreError) -> Self {
        let hint = match &e {
            CoreError::Config(_) => "Check project.yaml. Run `verimodel validate-config --config <file>` or print the schema with `--schema`.",
            CoreError::Io(_) => "Check that the file exists and that you have permission to read or write it.",
            CoreError::RulePack(_) => "Check the custom rule pack file referenced from project.yaml.",
            _ => "",
        };
        CliError::with_hint(e.to_string(), hint)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::with_hint(format!("File problem: {e}"), "Check the path and your permissions.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_error_and_hint() {
        let e = CliError::with_hint("boom", "try again");
        assert_eq!(e.render(), "Error: boom\nHint: try again");
        assert_eq!(CliError::new("boom").render(), "Error: boom");
        assert_eq!(CliError::with_hint("x", "  ").hint, None);
    }

    #[test]
    fn core_config_error_has_hint() {
        let e: CliError = CoreError::Config("bad".into()).into();
        assert!(e.message.contains("bad"));
        assert!(e.hint.is_some());
        assert!(!e.render().contains("Config("));
    }
}
