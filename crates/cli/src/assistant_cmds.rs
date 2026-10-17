//! `verimodel ask`, `verimodel assistant status|download` and `verimodel docs-to-yaml`.
//!
//! The assistant is an optional, local, read-only copilot. Nothing here changes a verdict,
//! a score or a quantity; `ask` only prints text.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use verimodel_assistant::model::{self, DEFAULT_MODEL};
use verimodel_assistant::{Answer, Assistant, AssistantError, Knowledge, Task};
use verimodel_core::model::RunReport;

use crate::error::CliError;

/// Same identifier as the desktop app, so the CLI and the app share one downloaded model.
const APP_IDENTIFIER: &str = "io.verimodel.desktop";

#[derive(Args, Debug)]
pub struct AppDataArg {
    /// Folder that holds the downloaded model (<dir>/assistant/<model>/). Default: the desktop app's data folder.
    #[arg(long, value_name = "DIR")]
    pub app_data: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct AskArgs {
    /// Your question, in plain language.
    #[arg(long, value_name = "TEXT")]
    pub question: String,
    #[command(flatten)]
    pub data: AppDataArg,
    /// report.json of a run; the assistant can then answer questions about that report.
    #[arg(long, value_name = "FILE")]
    pub report: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum AssistantCommand {
    /// Show whether the model is installed, its size, license and RAM need.
    Status(AppDataArg),
    /// Download the model once (about 0.5 GB from Hugging Face) into the app data folder.
    Download(AppDataArg),
}

#[derive(Args, Debug)]
pub struct DocsToYamlArgs {
    /// Files or folders to convert (.md .txt .yaml .json .docx .pdf).
    #[arg(long = "in", value_name = "PATH", num_args = 1.., required = true)]
    pub inputs: Vec<PathBuf>,
    /// knowledge.yaml to write.
    #[arg(long, value_name = "FILE", default_value = "knowledge.yaml")]
    pub out: PathBuf,
}

// ---------------------------------------------------------------------------------------------
// Plain helpers (unit-tested)
// ---------------------------------------------------------------------------------------------

/// The OS data folder of the desktop app. `env` looks up an environment variable.
/// `VERIMODEL_APP_DATA` overrides everything (handy for tests and portable installs).
pub fn default_app_data(env: &dyn Fn(&str) -> Option<String>, os: &str) -> PathBuf {
    if let Some(p) = env("VERIMODEL_APP_DATA").filter(|s| !s.trim().is_empty()) {
        return PathBuf::from(p);
    }
    let home = env("HOME")
        .or_else(|| env("USERPROFILE"))
        .unwrap_or_else(|| ".".into());
    let base = match os {
        "windows" => env("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&home).join("AppData").join("Roaming")),
        "macos" => PathBuf::from(&home)
            .join("Library")
            .join("Application Support"),
        _ => env("XDG_DATA_HOME")
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&home).join(".local").join("share")),
    };
    base.join(APP_IDENTIFIER)
}

fn app_data_dir(arg: &AppDataArg) -> PathBuf {
    arg.app_data
        .clone()
        .unwrap_or_else(|| default_app_data(&|k| std::env::var(k).ok(), std::env::consts::OS))
}

/// Text printed for an answer.
pub fn format_answer(answer: &Answer) -> String {
    match answer {
        Answer::Text {
            text,
            sources,
            structured,
        } => {
            let mut out = String::new();
            if !structured {
                out.push_str(
                    "Note: the assistant could not give a structured answer; shown as text.\n",
                );
            }
            out.push_str(text.trim());
            out.push('\n');
            if !sources.is_empty() {
                out.push_str(&format!("\nSources: {}\n", sources.join("; ")));
            }
            out
        }
        Answer::Explanation { text } => format!("{}\n", text.trim()),
        Answer::ConfigSnippet {
            yaml,
            valid,
            validation_error,
            explanation,
        } => {
            let mut out = format!("{}\n\n{}\n", explanation.trim(), yaml.trim_end());
            if *valid {
                out.push_str("\n(valid project.yaml fragment)\n");
            } else {
                out.push_str(&format!(
                    "\n(INVALID: {})\n",
                    validation_error
                        .clone()
                        .unwrap_or_else(|| "unknown problem".into())
                ));
            }
            out
        }
        Answer::Filter { filter } => {
            let mut parts = Vec::new();
            if !filter.severity.is_empty() {
                parts.push(format!("severity: {}", filter.severity.join(", ")));
            }
            for (k, v) in [
                ("rule", &filter.rule_id),
                ("storey", &filter.storey),
                ("type", &filter.ifc_type),
                ("search", &filter.search),
            ] {
                if let Some(v) = v {
                    parts.push(format!("{k}: {v}"));
                }
            }
            let mut out = format!(
                "{}\nFilter: {}\n",
                filter.explanation.trim(),
                if parts.is_empty() {
                    "(none)".into()
                } else {
                    parts.join(" | ")
                }
            );
            if !filter.dropped.is_empty() {
                out.push_str(&format!(
                    "Ignored (not in this report): {}\n",
                    filter.dropped.join(", ")
                ));
            }
            out
        }
    }
}

/// Knowledge for the CLI: built-in docs plus, when given, the run report.
pub fn build_knowledge(report: Option<&RunReport>) -> Knowledge {
    let mut k = Knowledge::builtin();
    if let Some(r) = report {
        k.add_report(r);
    }
    k
}

fn read_report(path: &Path) -> Result<RunReport, CliError> {
    if !path.is_file() {
        return Err(CliError::with_hint(
            format!("Report not found: {}", path.display()),
            "Use the report.json inside the out folder of a previous `verimodel check`.",
        ));
    }
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| {
        CliError::with_hint(
            format!("{} is not a VeriModel report.json: {e}", path.display()),
            "Use the report.json written by `verimodel check`.",
        )
    })
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

/// Ask a question. Fails with the plain-language hint (exit code 2) when the model is not installed.
pub fn ask(
    app_data: &Path,
    question: &str,
    report: Option<&RunReport>,
) -> Result<Answer, CliError> {
    if !model::status(app_data).installed {
        return Err(AssistantError::ModelNotInstalled.into());
    }
    let mut assistant = Assistant::load(app_data, build_knowledge(report))?;
    Ok(assistant.run(&Task::Ask {
        question: question.to_string(),
    })?)
}

pub fn cmd_ask(a: AskArgs) -> Result<i32, CliError> {
    if a.question.trim().is_empty() {
        return Err(CliError::with_hint(
            "The question is empty.",
            "Use --question \"What does rule CL-001 check?\".",
        ));
    }
    let report = match &a.report {
        Some(p) => Some(read_report(p)?),
        None => None,
    };
    let answer = ask(&app_data_dir(&a.data), &a.question, report.as_ref())?;
    print!("{}", format_answer(&answer));
    eprintln!(
        "\nRead-only copilot. Verdicts, scores and quantities never come from the assistant."
    );
    Ok(0)
}

pub fn render_status(s: &model::ModelStatus) -> String {
    let gb = s.gguf_size_bytes as f64 / 1_000_000_000.0;
    format!(
        "Model:     {}\nInstalled: {}\nSize:      {:.2} GB download\nLicense:   {}\nRAM:       about {} MB\nBackend:   {}\nFolder:    {}\n",
        s.name,
        if s.installed { "yes" } else { "no (run `verimodel assistant download`)" },
        gb,
        s.license,
        s.approx_ram_mb,
        s.backend,
        s.model_dir.display()
    )
}

pub fn cmd_assistant(c: AssistantCommand) -> Result<i32, CliError> {
    match c {
        AssistantCommand::Status(d) => {
            print!("{}", render_status(&model::status(&app_data_dir(&d))));
            Ok(0)
        }
        AssistantCommand::Download(d) => {
            let dir = app_data_dir(&d);
            let before = model::status(&dir);
            if before.installed {
                println!(
                    "The model is already installed in {}.",
                    before.model_dir.display()
                );
                return Ok(0);
            }
            println!(
                "Downloading {} ({:.2} GB, {}) from Hugging Face. This happens once.",
                DEFAULT_MODEL.name,
                DEFAULT_MODEL.gguf_size_bytes as f64 / 1e9,
                DEFAULT_MODEL.license
            );
            let mut last_pct = u64::MAX;
            let mut progress = |done: u64, total: Option<u64>| {
                let total = total.unwrap_or(DEFAULT_MODEL.gguf_size_bytes).max(1);
                let pct = (done * 100 / total).min(100);
                if pct != last_pct {
                    last_pct = pct;
                    eprint!(
                        "\r  {pct:>3}%  ({:.0} / {:.0} MB)",
                        done as f64 / 1e6,
                        total as f64 / 1e6
                    );
                    let _ = std::io::stderr().flush();
                }
            };
            let status = model::download(&dir, &DEFAULT_MODEL, &mut progress)?;
            eprintln!();
            print!("{}", render_status(&status));
            Ok(0)
        }
    }
}

pub fn cmd_docs_to_yaml(a: DocsToYamlArgs) -> Result<i32, CliError> {
    let r = verimodel_assistant::convert::convert_to_file(&a.inputs, &a.out)?;
    println!("Files read: {}", r.files_read.len());
    for f in &r.files_read {
        println!("  {f}");
    }
    println!("Skipped:    {}", r.skipped.len());
    for f in &r.skipped {
        println!("  {f}");
    }
    println!("Chunks:     {}", r.knowledge.chunks.len());
    println!("Written:    {}", a.out.display());
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn default_app_data_follows_each_os_convention() {
        assert_eq!(
            default_app_data(&env_of(&[("HOME", "/home/a")]), "linux"),
            PathBuf::from("/home/a/.local/share/io.verimodel.desktop")
        );
        assert_eq!(
            default_app_data(
                &env_of(&[("HOME", "/home/a"), ("XDG_DATA_HOME", "/x")]),
                "linux"
            ),
            PathBuf::from("/x/io.verimodel.desktop")
        );
        assert_eq!(
            default_app_data(&env_of(&[("HOME", "/Users/a")]), "macos"),
            PathBuf::from("/Users/a/Library/Application Support/io.verimodel.desktop")
        );
        assert_eq!(
            default_app_data(&env_of(&[("APPDATA", "C:\\R")]), "windows"),
            PathBuf::from("C:\\R").join("io.verimodel.desktop")
        );
        assert_eq!(
            default_app_data(
                &env_of(&[("VERIMODEL_APP_DATA", "/d"), ("HOME", "/h")]),
                "linux"
            ),
            PathBuf::from("/d")
        );
    }

    #[test]
    fn ask_without_model_fails_with_the_plain_hint() {
        let dir = tempfile::tempdir().unwrap();
        let e = ask(dir.path(), "what is IDS?", None).unwrap_err();
        let text = e.render();
        assert!(
            text.starts_with("Error: The assistant model is not installed yet."),
            "{text}"
        );
        assert!(text.contains("Hint:") && text.contains("Download model"));
    }

    #[test]
    fn answers_are_printed_as_plain_text() {
        let a = Answer::Text {
            text: " Hello ".into(),
            sources: vec!["docs/RULES.md".into(), "docs/IDS.md".into()],
            structured: true,
        };
        assert_eq!(
            format_answer(&a),
            "Hello\n\nSources: docs/RULES.md; docs/IDS.md\n"
        );
        let a = Answer::Text {
            text: "x".into(),
            sources: vec![],
            structured: false,
        };
        assert!(
            format_answer(&a).starts_with("Note: the assistant could not give a structured answer")
        );
        let a = Answer::ConfigSnippet {
            yaml: "a: 1\n".into(),
            valid: false,
            validation_error: Some("bad".into()),
            explanation: "e".into(),
        };
        assert!(format_answer(&a).contains("INVALID: bad"));
        let f = verimodel_assistant::FilterSpec {
            severity: vec!["ERROR".into()],
            storey: Some("Level 1".into()),
            dropped: vec!["Level 9".into()],
            explanation: "ok".into(),
            ..Default::default()
        };
        let out = format_answer(&Answer::Filter { filter: f });
        assert!(
            out.contains("severity: ERROR | storey: Level 1")
                && out.contains("Ignored (not in this report): Level 9")
        );
    }

    #[test]
    fn knowledge_gets_the_report_chunks_only_when_given() {
        let base = build_knowledge(None).chunks.len();
        let mut r = RunReport::default();
        r.model.file_name = "x.ifc".into();
        let with = build_knowledge(Some(&r));
        assert!(with.chunks.len() > base);
        assert!(with.chunks.iter().any(|c| c.source == "report:x.ifc"));
    }

    #[test]
    fn parses_the_new_subcommands() {
        use crate::{Cli, Command};
        let cli = Cli::try_parse_from([
            "verimodel",
            "ask",
            "--question",
            "q",
            "--app-data",
            "/d",
            "--report",
            "r.json",
        ])
        .unwrap();
        match cli.command {
            Command::Ask(a) => assert_eq!(
                (a.question.as_str(), a.data.app_data, a.report),
                (
                    "q",
                    Some(PathBuf::from("/d")),
                    Some(PathBuf::from("r.json"))
                )
            ),
            _ => panic!(),
        }
        let cli = Cli::try_parse_from([
            "verimodel",
            "docs-to-yaml",
            "--in",
            "a.md",
            "dir",
            "--out",
            "k.yaml",
        ])
        .unwrap();
        match cli.command {
            Command::DocsToYaml(a) => assert_eq!(
                (a.inputs, a.out),
                (
                    vec![PathBuf::from("a.md"), PathBuf::from("dir")],
                    PathBuf::from("k.yaml")
                )
            ),
            _ => panic!(),
        }
        assert!(Cli::try_parse_from(["verimodel", "assistant", "status"]).is_ok());
        assert!(
            Cli::try_parse_from(["verimodel", "assistant", "download", "--app-data", "/d"]).is_ok()
        );
        assert!(
            Cli::try_parse_from(["verimodel", "docs-to-yaml"]).is_err(),
            "--in is required"
        );
    }

    #[test]
    fn missing_report_is_a_clean_error() {
        let e = read_report(Path::new("/no/such/report.json")).unwrap_err();
        assert!(e.render().starts_with("Error: Report not found"));
    }
}
