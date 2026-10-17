//! Append-only audit log (`audit.log`): one line per event, never rewritten.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::CoreResult;

/// Writer for `audit.log`. Each line: `<ISO timestamp>\t<event>\t<details>`.
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn open(dir: &Path) -> CoreResult<AuditLog> {
        std::fs::create_dir_all(dir)?;
        Ok(AuditLog { path: dir.join("audit.log") })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one event. Errors are returned, never swallowed, so a run cannot silently lose its trail.
    pub fn record(&self, event: &str, details: &str) -> CoreResult<()> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        writeln!(f, "{ts}\t{event}\t{}", details.replace('\n', " "))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appends_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::open(dir.path()).unwrap();
        log.record("start", "a").unwrap();
        log.record("end", "b\nc").unwrap();
        let text = std::fs::read_to_string(log.path()).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with("end\tb c"));
    }
}
