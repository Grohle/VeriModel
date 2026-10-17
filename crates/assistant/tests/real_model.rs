//! Smoke test against the real default model (Qwen2.5-0.5B-Instruct Q4_K_M).
//!
//! Ignored by default. Run with a folder holding `model.gguf` (or the original
//! `qwen2.5-0.5b-instruct-q4_k_m.gguf`) and `tokenizer.json`:
//!
//! ```sh
//! VERIMODEL_ASSISTANT_MODEL_DIR=/path/to/models \
//!   cargo test -p verimodel-assistant --release --test real_model -- --ignored --nocapture
//! ```
//!
//! Use a release build: debug inference is far too slow.
#![cfg(feature = "candle")]

use std::path::{Path, PathBuf};
use std::time::Instant;

use verimodel_assistant::backend::candle::CandleBackend;
use verimodel_assistant::backend::{Backend, GenOptions};
use verimodel_assistant::model::{sha256_file, DEFAULT_MODEL};
use verimodel_assistant::tasks::{self, Answer, ReportContext, Task};
use verimodel_assistant::AssistantResult;
use verimodel_assistant::Knowledge;
use verimodel_core::model::{Finding, RuleResult, RuleSource, RuleStatus, Severity};

fn find_model_files(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let tok = dir.join("tokenizer.json");
    let gguf = [
        dir.join("model.gguf"),
        dir.join(format!("{}.gguf", DEFAULT_MODEL.id)),
    ]
    .into_iter()
    .find(|p| p.is_file())?;
    tok.is_file().then_some((gguf, tok))
}

/// Resident memory of this process in MB: (current, peak).
fn rss_mb() -> (u64, u64) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            / 1024
    };
    (field("VmRSS:"), field("VmHWM:"))
}

/// Prints every raw model output (and its timing) before the task parses it.
struct Logged<'a>(&'a mut CandleBackend);

impl Backend for Logged<'_> {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn generate(&mut self, prompt: &str, opts: &GenOptions) -> AssistantResult<String> {
        let out = self.0.generate(prompt, opts)?;
        let s = self.0.last_stats().unwrap_or_default();
        println!(
            "    raw output ({} prompt tokens, prefill {:.2} s = {:.0} tok/s; {} generated, decode {:.2} s = {:.1} tok/s): {}",
            s.prompt_tokens,
            s.prefill_secs,
            s.prefill_tokens_per_sec(),
            s.generated_tokens,
            s.decode_secs,
            s.decode_tokens_per_sec(),
            out.trim()
        );
        Ok(out)
    }
}

fn run(backend: &mut CandleBackend, knowledge: &Knowledge, label: &str, task: &Task) -> Answer {
    println!("\n=== {label}");
    let t = Instant::now();
    let answer = tasks::run(&mut Logged(backend), knowledge, task).expect("generation failed");
    println!("    total {:.1} s", t.elapsed().as_secs_f64());
    println!("{}", serde_json::to_string_pretty(&answer).unwrap());
    answer
}

#[test]
#[ignore = "needs VERIMODEL_ASSISTANT_MODEL_DIR and a release build"]
fn real_model_smoke() {
    let Some(dir) = std::env::var_os("VERIMODEL_ASSISTANT_MODEL_DIR") else {
        panic!("set VERIMODEL_ASSISTANT_MODEL_DIR to a folder with model.gguf and tokenizer.json");
    };
    let (gguf, tok) = find_model_files(Path::new(&dir))
        .expect("model.gguf / tokenizer.json not found in VERIMODEL_ASSISTANT_MODEL_DIR");

    // Verify the files against the catalogue checksums (install_from_files does the same
    // after copying; it is unit-tested in model.rs, so the 0.5 GB copy is skipped here).
    let t = Instant::now();
    assert_eq!(
        sha256_file(&gguf).unwrap(),
        DEFAULT_MODEL.gguf_sha256,
        "model.gguf checksum"
    );
    assert_eq!(
        sha256_file(&tok).unwrap(),
        DEFAULT_MODEL.tokenizer_sha256,
        "tokenizer.json checksum"
    );
    println!("SHA-256 verification: {:.1} s", t.elapsed().as_secs_f64());

    let (rss0, _) = rss_mb();
    let t = Instant::now();
    let mut backend = CandleBackend::load(&gguf, &tok).expect("load");
    let (rss1, _) = rss_mb();
    println!(
        "load: {:.2} s, family {}, RSS {} MB -> {} MB",
        t.elapsed().as_secs_f64(),
        backend.family(),
        rss0,
        rss1
    );
    assert_eq!(backend.family(), "qwen2");

    let knowledge = Knowledge::builtin();
    let mut failures: Vec<String> = Vec::new();

    // (a) Ask
    let a = run(
        &mut backend,
        &knowledge,
        "Ask: What does rule ID-001 check?",
        &Task::Ask {
            question: "What does rule ID-001 check?".into(),
        },
    );
    match &a {
        Answer::Text {
            text, structured, ..
        } => {
            if !text.to_lowercase().contains("globalid") {
                failures.push(format!("(a) answer does not mention GlobalId: {text}"));
            }
            if !structured {
                failures.push("(a) answer was not valid JSON (plain-text fallback)".into());
            }
        }
        other => failures.push(format!("(a) unexpected answer kind: {other:?}")),
    }

    // (b) Filter
    let context = ReportContext {
        rule_ids: vec!["ID-001".into(), "SP-001".into()],
        storeys: vec!["Level 1".into(), "Level 2".into()],
        ifc_types: vec!["IfcWall".into(), "IfcSlab".into()],
        severities: vec!["ERROR".into(), "WARNING".into(), "INFO".into()],
    };
    let b = run(
        &mut backend,
        &knowledge,
        "Filter: show me the errors on Level 1 for walls",
        &Task::Filter {
            question: "show me the errors on Level 1 for walls".into(),
            context,
        },
    );
    match &b {
        Answer::Filter { filter } => {
            if filter.severity != vec!["ERROR".to_string()]
                || filter.storey.as_deref() != Some("Level 1")
                || filter.ifc_type.as_deref() != Some("IfcWall")
            {
                failures.push(format!("(b) wrong filter: {filter:?}"));
            }
        }
        other => failures.push(format!("(b) unexpected answer kind: {other:?}")),
    }

    // (c) Explain
    let finding = Finding {
        rule_id: "SP-001".into(),
        severity: Severity::Error,
        global_id: Some("1hOSvn6df7F8_7GcBWlRGQ".into()),
        step_id: Some(1234),
        ifc_type: Some("IfcWall".into()),
        name: Some("Basic Wall:Exterior 300".into()),
        storey: None,
        message: "Element is not contained in any spatial structure element".into(),
        expected: Some("contained in a storey, space, site or facility part".into()),
        actual: Some("no spatial container".into()),
        fix_hint: Some(
            "Assign the element to a level / storey in the authoring tool before exporting.".into(),
        ),
        source: RuleSource::RulePack {
            pack: "generic".into(),
        },
    };
    let rule = RuleResult {
        rule_id: "SP-001".into(),
        category: "spatial".into(),
        description: "Every element must be contained in a spatial structure element (storey, space, site, facility part)".into(),
        severity: Severity::Error,
        status: RuleStatus::Fail,
        elements_checked: 412,
        failures: 3,
        skipped_reason: None,
        source: RuleSource::RulePack { pack: "generic".into() },
    };
    let c = run(
        &mut backend,
        &knowledge,
        "Explain: SP-001 finding",
        &Task::Explain {
            finding: Box::new(finding),
            rule: Some(rule),
        },
    );
    match &c {
        Answer::Explanation { text } | Answer::Text { text, .. } => {
            let t = text.to_lowercase();
            if text.trim().is_empty()
                || !(t.contains("basic wall") || t.contains("1hosvn6df7f8_7gcbwlrgq"))
            {
                failures.push(format!(
                    "(c) explanation does not mention the element: {text}"
                ));
            }
            if matches!(c, Answer::Text { .. }) {
                failures.push("(c) explanation was not valid JSON (plain-text fallback)".into());
            }
        }
        other => failures.push(format!("(c) unexpected answer kind: {other:?}")),
    }

    // (d) ConfigHelp
    let d = run(
        &mut backend,
        &knowledge,
        "ConfigHelp: require FireRating on all walls",
        &Task::ConfigHelp {
            question: "require FireRating on all walls".into(),
            current_yaml: None,
        },
    );
    match &d {
        Answer::ConfigSnippet {
            yaml,
            valid,
            validation_error,
            ..
        } => {
            if !(yaml.contains("Pset_WallCommon") && yaml.contains("FireRating") && *valid) {
                failures.push(format!(
                    "(d) wrong snippet (valid {valid}, error {validation_error:?}):\n{yaml}"
                ));
            }
        }
        other => failures.push(format!("(d) unexpected answer kind: {other:?}")),
    }

    // Determinism: greedy decoding gives the same answer twice.
    let d2 = tasks::run(
        &mut backend,
        &knowledge,
        &Task::ConfigHelp {
            question: "require FireRating on all walls".into(),
            current_yaml: None,
        },
    )
    .unwrap();
    if d2 != d {
        failures.push("(d) not deterministic".into());
    }

    let (rss, peak) = rss_mb();
    println!("\nRSS after the run: {rss} MB (peak {peak} MB)");
    assert!(
        failures.is_empty(),
        "failed checks:\n{}",
        failures.join("\n")
    );
}
