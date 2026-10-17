//! # VeriModel core
//!
//! Pure Rust engine: rule engine, quantity takeoff, BCF writer, report
//! writers, audit ledger and configuration. It never touches an IFC file:
//! the `verimodel-ifc-reader` crate produces a [`model::Model`] and this crate
//! turns it into verdicts and outputs. Everything here is unit-testable with
//! synthetic structs.
//!
//! Module map:
//! * [`model`] — shared facts/verdict types (`Element`, `Finding`, `RunReport`, ...)
//! * [`config`] — `project.yaml`
//! * [`rules`] — pluggable rule engine (rule packs are YAML data)
//! * [`score`] — score and verdict formula
//! * [`ids`] — turns IDS engine results into rules/findings, quoting requirements
//! * [`qto`] — quantity takeoff aggregation and cost estimate
//! * [`bcf`] — BCF 2.1 / 3.0 writer
//! * [`report`] — report.html / report.json / failures.csv / qto.csv / qto.xlsx
//! * [`clash`] — clash detection (phase 2 placeholder)
//! * [`run`] — orchestrates one run and writes the `out/` folder
//! * [`audit`], [`hash`], [`disciplines`], [`i18n`] — support

pub mod audit;
pub mod bcf;
pub mod clash;
pub mod config;
pub mod disciplines;
pub mod error;
pub mod hash;
pub mod i18n;
pub mod ids;
pub mod model;
pub mod qto;
pub mod report;
pub mod rules;
pub mod run;
pub mod score;

pub use config::Config;
pub use error::{CoreError, CoreResult};
pub use model::{Model, RunReport};

/// Product name, used in window titles, reports and BCF metadata.
pub const TOOL_NAME: &str = "VeriModel";
/// Crate version (same number for CLI, desktop app and installers).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
