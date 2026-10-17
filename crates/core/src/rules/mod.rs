//! Pluggable rule engine.
//!
//! A **rule** is data (YAML): id, description, scope, a *check kind* with
//! parameters, severity and a fix hint. The engine knows a fixed vocabulary of
//! check kinds (see [`checks::CheckKind`]); adding a rule never requires
//! editing engine code, only adding a YAML entry. Project specific values are
//! referenced with `$config.<dotted.path>` and resolved against `project.yaml`
//! at evaluation time.

pub mod checks;
pub mod engine;
pub mod pack;

pub use engine::{evaluate, Evaluation};
pub use pack::{load_rule_pack, Rule, RulePack, RuleScope};
