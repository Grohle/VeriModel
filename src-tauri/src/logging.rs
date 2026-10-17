//! A tiny file logger (no extra dependency). Technical detail (what the UI must
//! not show) lands in `<app log dir>/verimodel.log`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};

struct FileLogger {
    file: Mutex<File>,
    level: LevelFilter,
}

impl Log for FileLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= self.level
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {} - {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            r.level(),
            r.target(),
            r.args()
        );
        if cfg!(debug_assertions) {
            eprint!("{line}");
        }
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

/// Install the logger writing to `<dir>/verimodel.log`. Returns the log path.
/// Failing to log never stops the app: on error the app simply runs without a log file.
pub fn init(dir: &Path) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join("verimodel.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok()?;
    let level = if cfg!(debug_assertions) { LevelFilter::Debug } else { LevelFilter::Info };
    let logger = FileLogger { file: Mutex::new(file), level };
    log::set_boxed_logger(Box::new(logger)).ok()?;
    log::set_max_level(level);
    Some(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn init_creates_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = super::init(&dir.path().join("logs"));
        // set_boxed_logger can only succeed once per process; both outcomes are fine.
        if let Some(p) = p {
            log::info!("hello from test");
            log::logger().flush();
            assert!(p.is_file());
        }
    }
}
