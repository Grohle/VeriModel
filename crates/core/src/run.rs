//! One run, end to end: evaluate rules, IDS, QTO, clash; write the `out/` folder.
//!
//! The reader (another crate) produces the [`Model`]; this module does the
//! rest and is shared by the CLI and the desktop app. `report.json`,
//! `failures.csv` and `qto.csv` are deterministic; only `run.json` and
//! `audit.log` carry timestamps.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::audit::AuditLog;
use crate::config::Config;
use crate::error::{CoreError, CoreResult};
use crate::hash;
use crate::i18n::Strings;
use crate::model::{Model, RunLedger, RunReport, ToolInfo};
use crate::report::OutputFile;
use crate::rules::{self, engine};
use crate::{bcf, clash, ids, qto, report};

/// What the caller hands to [`execute`].
#[derive(Debug, Clone, Default)]
pub struct RunInputs {
    /// Path of the IFC file (for the ledger; the model is already read).
    pub ifc_path: PathBuf,
    pub ids_path: Option<PathBuf>,
    /// `project.yaml` path if one was given; `None` means defaults were used.
    pub config_path: Option<PathBuf>,
    /// Directory the config was loaded from (relative paths resolve against it).
    pub config_dir: Option<PathBuf>,
    pub out_dir: PathBuf,
    /// Moment the reader started, so `duration_ms` covers reading too.
    pub started: Option<Instant>,
}

/// Everything a run produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOutcome {
    pub report: RunReport,
    pub ledger: RunLedger,
    pub files: Vec<OutputFile>,
    pub bcf: bcf::BcfStats,
    pub out_dir: PathBuf,
}

/// Build the report without writing anything (used by tests and the UI preview).
pub fn build_report(model: &Model, config: &Config, config_dir: Option<&Path>) -> CoreResult<RunReport> {
    let pack = rules::load_rule_pack(config, config_dir)?;
    let mut eval = engine::evaluate(model, config, &pack, config_dir)?;

    let ids_report = match &model.ids_results {
        Some(results) => {
            let mut ev = ids::evaluate(results);
            // The IDS engine reports GlobalIds only; add storey and step id from the model
            // so IDS findings carry the same evidence as rule-pack findings.
            let index: std::collections::BTreeMap<&str, &crate::model::Element> =
                model.elements.iter().map(|e| (e.global_id.as_str(), e)).collect();
            for f in &mut ev.findings {
                if let Some(e) = f.global_id.as_deref().and_then(|g| index.get(g)) {
                    if f.storey.is_none() {
                        f.storey = e.storey.clone();
                    }
                    if f.step_id.is_none() {
                        f.step_id = Some(e.step_id);
                    }
                    if f.ifc_type.is_none() {
                        f.ifc_type = Some(e.ifc_type.clone());
                    }
                }
            }
            engine::merge(&mut eval, ev.rules, ev.findings);
            Some(ev.report)
        }
        None => None,
    };

    let rates = match &config.qto.unit_rates_file {
        Some(f) => Some(qto::RateTable::from_csv(&Config::resolve_path(config_dir, f))?),
        None => None,
    };
    let qto_report = qto::compute(model, config, rates.as_ref())?;
    let strings = Strings::for_language(config.output.language);
    let clash_report = clash::run(model, None, config, &strings);

    let mut report = RunReport {
        tool: ToolInfo::default(),
        model: model.info.clone(),
        config: config.to_json_value(),
        summary: eval.summary,
        rules: eval.rules,
        findings: eval.findings,
        qto: qto_report,
        ids: ids_report,
        clash: clash_report,
    };
    report.normalise();
    Ok(report)
}

/// Evaluate the model and write the whole `out/` folder.
pub fn execute(model: &Model, config: &Config, inputs: &RunInputs) -> CoreResult<RunOutcome> {
    let started = inputs.started.unwrap_or_else(Instant::now);
    std::fs::create_dir_all(&inputs.out_dir)?;
    let audit = AuditLog::open(&inputs.out_dir)?;
    audit.record("run.start", &format!("ifc={} schema={} elements={}", inputs.ifc_path.display(), model.info.schema, model.elements.len()))?;

    let report = build_report(model, config, inputs.config_dir.as_deref())?;
    audit.record(
        "rules.done",
        &format!("rules={} errors={} warnings={} score={}", report.rules.len(), report.summary.errors, report.summary.warnings, report.summary.score),
    )?;

    let mut files = report::write_all(&report, config, &inputs.out_dir)?;
    audit.record("report.written", &files.iter().map(|f| f.name.clone()).collect::<Vec<_>>().join(","))?;

    let bcf_path = inputs.out_dir.join("issues.bcfzip");
    let bcf_stats = bcf::write_bcfzip(&report, model, config, &bcf_path)?;
    files.push(OutputFile { name: "issues.bcfzip".into(), kind: "bcf".into(), bytes: file_len(&bcf_path) });
    let issues_csv = inputs.out_dir.join("issues.csv");
    bcf::write_issues_csv(&report, &issues_csv)?;
    files.push(OutputFile { name: "issues.csv".into(), kind: "issues_csv".into(), bytes: file_len(&issues_csv) });
    audit.record("bcf.written", &format!("topics={}", bcf_stats.topics))?;

    let cfg_yaml = config.to_yaml()?;
    std::fs::write(inputs.out_dir.join("project.yaml"), &cfg_yaml)?;
    files.push(OutputFile { name: "project.yaml".into(), kind: "config".into(), bytes: cfg_yaml.len() as u64 });

    let report_sha = hash::sha256_file(&inputs.out_dir.join("report.json"))?;
    let ledger = RunLedger {
        tool: ToolInfo::default(),
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        duration_ms: started.elapsed().as_millis() as u64,
        ifc_file: file_name(&inputs.ifc_path),
        ifc_sha256: model.info.sha256.clone(),
        ids_file: inputs.ids_path.as_ref().map(|p| file_name(p)),
        ids_sha256: match &inputs.ids_path {
            Some(p) => Some(hash::sha256_file(p)?),
            None => None,
        },
        config_file: inputs.config_path.as_ref().map(|p| file_name(p)),
        config_sha256: hash::sha256_bytes(cfg_yaml.as_bytes()),
        rates_file: config.qto.unit_rates_file.clone(),
        rates_sha256: match &config.qto.unit_rates_file {
            Some(f) => Some(hash::sha256_file(&Config::resolve_path(inputs.config_dir.as_deref(), f))?),
            None => None,
        },
        schema: model.info.schema.clone(),
        element_count: model.elements.len(),
        score: report.summary.score,
        verdict: report.summary.verdict,
        errors: report.summary.errors,
        warnings: report.summary.warnings,
        infos: report.summary.infos,
        rules_passed: report.summary.rules_passed,
        rules_failed: report.summary.rules_failed,
        rules_skipped: report.summary.rules_skipped,
        report_sha256: report_sha,
        output_dir: inputs.out_dir.display().to_string(),
    };
    let ledger_text = serde_json::to_string_pretty(&ledger)?;
    std::fs::write(inputs.out_dir.join("run.json"), &ledger_text)?;
    files.push(OutputFile { name: "run.json".into(), kind: "run_json".into(), bytes: ledger_text.len() as u64 });
    files.push(OutputFile { name: "audit.log".into(), kind: "audit_log".into(), bytes: file_len(audit.path()) });
    audit.record("run.end", &format!("duration_ms={} verdict={:?}", ledger.duration_ms, ledger.verdict))?;

    Ok(RunOutcome { report, ledger, files, bcf: bcf_stats, out_dir: inputs.out_dir.clone() })
}

/// Read a `run.json` ledger (for run history).
pub fn read_ledger(path: &Path) -> CoreResult<RunLedger> {
    let text = std::fs::read_to_string(path).map_err(|e| CoreError::Io(format!("cannot read {}: {e}", path.display())))?;
    Ok(serde_json::from_str(&text)?)
}

/// Scan a directory for runs (`<dir>/*/run.json`), newest first by timestamp.
pub fn list_runs(root: &Path) -> CoreResult<Vec<RunLedger>> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for entry in rd.flatten() {
            let p = entry.path().join("run.json");
            if p.is_file() {
                if let Ok(l) = read_ledger(&p) {
                    out.push(l);
                }
            }
        }
    }
    out.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(out)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Element, ModelInfo};

    fn tiny_model() -> Model {
        let mk = |id: &str, name: &str| Element {
            global_id: id.into(),
            ifc_type: "IfcWall".into(),
            ancestors: vec!["IfcWall".into(), "IfcElement".into(), "IfcProduct".into()],
            name: Some(name.into()),
            has_representation: true,
            ..Default::default()
        };
        let mut m = Model {
            info: ModelInfo { schema: "IFC4".into(), file_name: "t.ifc".into(), sha256: "00".into(), ..Default::default() },
            elements: vec![mk("0fqX614OH1YO1Njdxms2$Q", "A"), mk("0fqX614OH1YO1Njdxms2$Q", "B")],
            ids_results: None,
        };
        m.sort_elements();
        m
    }

    #[test]
    fn execute_writes_all_outputs_and_is_deterministic() {
        let model = tiny_model();
        let cfg = Config::default();
        let dir = tempfile::tempdir().unwrap();
        let run_once = |name: &str| {
            let out = dir.path().join(name);
            let inputs = RunInputs { ifc_path: "t.ifc".into(), out_dir: out.clone(), ..Default::default() };
            let o = execute(&model, &cfg, &inputs).unwrap();
            assert!(o.files.iter().any(|f| f.name == "report.json"));
            assert!(out.join("run.json").is_file());
            assert!(out.join("audit.log").is_file());
            assert!(out.join("project.yaml").is_file());
            std::fs::read(out.join("report.json")).unwrap()
        };
        let a = run_once("a");
        let b = run_once("b");
        assert_eq!(a, b, "report.json must be byte-identical across runs");
        let runs = list_runs(dir.path()).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].verdict, Some(crate::model::Verdict::Fail));
    }
}
