//! `verimodel` command line.
//!
//! Exit codes: 0 = PASS / PASS WITH WARNINGS (or a successful helper command),
//! 1 = verdict FAIL, 2 = error (bad input, missing file, missing IFC tooling),
//! 3 = feature planned but not implemented yet (`clash`).

mod error;
mod render;

use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use verimodel_core::config::Config;
use verimodel_core::i18n::Strings;
use verimodel_core::model::Model;
use verimodel_core::run::{self, RunInputs, RunOutcome};
use verimodel_ifc_reader::ReadOptions;

use error::{CliError, EXIT_ERROR, EXIT_NOT_IMPLEMENTED};

const ABOUT: &str = "VeriModel \u{2014} IFC QA Gate + Quantity Takeoff + BCF Export";

#[derive(Parser, Debug)]
#[command(name = "verimodel", version = verimodel_core::VERSION, about = ABOUT, long_about = ABOUT)]
#[command(after_help = "Exit codes: 0 pass (or pass with warnings), 1 fail, 2 error, 3 not implemented yet.")]
struct Cli {
    /// Print detailed logs to stderr (repeat for more: -vv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Check an IFC file against the rules, IDS and project settings; writes the out/ folder.
    Check(CheckArgs),
    /// Quantity takeoff: print the QTO table and write the outputs.
    Qto(QtoArgs),
    /// Clash detection between two models (planned for a later version).
    Clash(ClashArgs),
    /// Validate a project.yaml, or print its JSON schema.
    ValidateConfig(ValidateArgs),
    /// Show a quick file card of an IFC file (schema, units, storeys, element types).
    Inspect(InspectArgs),
    /// Check that Python, IfcOpenShell and IfcTester are installed.
    Doctor,
}

#[derive(Args, Debug)]
struct CommonRun {
    /// IFC file to read.
    #[arg(long, value_name = "FILE")]
    ifc: PathBuf,
    /// project.yaml with requirements, tolerances and QTO settings (defaults are used if omitted).
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Output folder (created if missing).
    #[arg(long, value_name = "DIR", default_value = "./out")]
    out: PathBuf,
    /// Skip geometry (faster; geometry rules are then reported as skipped).
    #[arg(long)]
    no_geometry: bool,
}

#[derive(Args, Debug)]
struct CheckArgs {
    #[command(flatten)]
    common: CommonRun,
    /// buildingSMART IDS file to audit the model against.
    #[arg(long, value_name = "FILE")]
    ids: Option<PathBuf>,
    /// Print the full run outcome as JSON instead of the text summary.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct QtoArgs {
    #[command(flatten)]
    common: CommonRun,
}

#[derive(Args, Debug)]
struct ClashArgs {
    /// First IFC file.
    #[arg(long, value_name = "FILE")]
    ifc_a: PathBuf,
    /// Second IFC file.
    #[arg(long, value_name = "FILE")]
    ifc_b: PathBuf,
    /// project.yaml with the clash sets.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Output folder.
    #[arg(long, value_name = "DIR", default_value = "./out")]
    out: PathBuf,
}

#[derive(Args, Debug)]
struct ValidateArgs {
    /// project.yaml to validate.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Print the JSON schema of project.yaml (for editor autocompletion).
    #[arg(long)]
    schema: bool,
}

#[derive(Args, Debug)]
struct InspectArgs {
    /// IFC file to inspect.
    #[arg(long, value_name = "FILE")]
    ifc: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    init_logging(cli.verbose);
    let code = match dispatch(cli.command) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{}", e.render());
            EXIT_ERROR
        }
    };
    std::process::exit(code);
}

fn init_logging(verbose: u8) {
    let level = match verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).try_init();
}

fn dispatch(cmd: Command) -> Result<i32, CliError> {
    match cmd {
        Command::Check(a) => cmd_check(a),
        Command::Qto(a) => cmd_qto(a),
        Command::Clash(a) => cmd_clash(a),
        Command::ValidateConfig(a) => cmd_validate(a),
        Command::Inspect(a) => cmd_inspect(a),
        Command::Doctor => cmd_doctor(),
    }
}

// ---------------------------------------------------------------------------------------------

fn require_file(path: &Path, what: &str) -> Result<(), CliError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(CliError::with_hint(
            format!("{what} not found: {}", path.display()),
            "Check the path (it is relative to the folder you run the command from).",
        ))
    }
}

/// Load the config and the directory relative paths inside it resolve against.
fn load_config(path: Option<&Path>) -> Result<(Config, Option<PathBuf>), CliError> {
    match path {
        Some(p) => {
            require_file(p, "Config file")?;
            let cfg = Config::from_file(p)?;
            let dir = match p.parent() {
                Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
                _ => PathBuf::from("."),
            };
            Ok((cfg, Some(dir)))
        }
        None => Ok((Config::default(), None)),
    }
}

/// Shared pipeline of `check` and `qto`: read, evaluate, write.
fn run_pipeline(common: &CommonRun, ids: Option<&PathBuf>) -> Result<RunOutcome, CliError> {
    let started = Instant::now();
    require_file(&common.ifc, "IFC file")?;
    let (config, config_dir) = load_config(common.config.as_deref())?;
    // --ids wins; otherwise use requirements.ids_file from project.yaml (relative to the config).
    let ids: Option<PathBuf> = ids.cloned().or_else(|| config.ids_path(config_dir.as_deref()));
    if let Some(ids) = &ids {
        require_file(ids, "IDS file")?;
        log::info!("using IDS file {}", ids.display());
    }
    let opts = ReadOptions { ids_path: ids.clone(), geometry: !common.no_geometry, ..Default::default() };
    log::info!("reading {}", common.ifc.display());
    let model = verimodel_ifc_reader::read_model(&common.ifc, &opts)?;
    log::info!("read {} elements", model.elements.len());
    let inputs = RunInputs {
        ifc_path: common.ifc.clone(),
        ids_path: ids,
        config_path: common.config.clone(),
        config_dir,
        out_dir: common.out.clone(),
        started: Some(started),
    };
    Ok(run::execute(&model, &config, &inputs)?)
}

fn cmd_check(a: CheckArgs) -> Result<i32, CliError> {
    let outcome = run_pipeline(&a.common, a.ids.as_ref())?;
    if a.json {
        let text = serde_json::to_string_pretty(&outcome).map_err(|e| CliError::new(format!("Cannot render JSON: {e}")))?;
        println!("{text}");
    } else {
        print!("{}", render::render_summary(&outcome));
    }
    Ok(render::exit_code_for(outcome.report.summary.verdict))
}

fn cmd_qto(a: QtoArgs) -> Result<i32, CliError> {
    let outcome = run_pipeline(&a.common, None)?;
    print!("{}", render::render_qto(&outcome.report.qto));
    println!("\nOutput folder: {}", outcome.out_dir.display());
    for f in &outcome.files {
        if f.name.starts_with("qto") {
            println!("  {}", f.name);
        }
    }
    // QTO is informational: a failing QA verdict does not make the takeoff fail.
    Ok(0)
}

fn cmd_clash(a: ClashArgs) -> Result<i32, CliError> {
    require_file(&a.ifc_a, "First IFC file")?;
    require_file(&a.ifc_b, "Second IFC file")?;
    let (mut config, _) = load_config(a.config.as_deref())?;
    // The user asked for clashes explicitly, so report "not implemented", not "disabled".
    config.clash.enabled = true;
    let strings = Strings::for_language(config.output.language);
    let report = verimodel_core::clash::run(&Model::default(), Some(&Model::default()), &config, &strings);
    println!("{}", report.message);
    if !report.implemented {
        eprintln!("No clash results were produced. Exit code {EXIT_NOT_IMPLEMENTED} means 'not implemented yet'.");
        return Ok(EXIT_NOT_IMPLEMENTED);
    }
    Ok(0)
}

fn cmd_validate(a: ValidateArgs) -> Result<i32, CliError> {
    if a.schema {
        let schema = Config::json_schema();
        let text = serde_json::to_string_pretty(&schema).map_err(|e| CliError::new(format!("Cannot render schema: {e}")))?;
        println!("{text}");
        return Ok(0);
    }
    let path = a.config.ok_or_else(|| CliError::with_hint("No config file given.", "Use --config <project.yaml>, or --schema to print the schema."))?;
    require_file(&path, "Config file")?;
    Config::from_file(&path)?;
    println!("OK: {} is a valid VeriModel configuration.", path.display());
    Ok(0)
}

fn cmd_inspect(a: InspectArgs) -> Result<i32, CliError> {
    require_file(&a.ifc, "IFC file")?;
    let info = verimodel_ifc_reader::inspect(&a.ifc)?;
    print!("{}", render::render_info(&info));
    Ok(0)
}

fn cmd_doctor() -> Result<i32, CliError> {
    match verimodel_ifc_reader::check_environment() {
        Ok(env) => {
            println!("Python:       {} ({})", env.python, env.python_version);
            println!("IfcOpenShell: {}", env.ifcopenshell_version);
            if env.ifctester {
                println!("IfcTester:    installed (IDS checks available)");
                println!("\nEverything needed is installed.");
                Ok(0)
            } else {
                println!("IfcTester:    NOT installed (IDS checks unavailable)");
                println!("\nIFC reading works. To enable IDS checks run:\n  pip install ifcopenshell ifctester");
                Ok(0)
            }
        }
        Err(e) => {
            let mut err = CliError::from(e);
            let install = "Install the IFC tooling with: pip install ifcopenshell ifctester";
            err.hint = Some(match err.hint.take() {
                Some(h) if !h.contains("pip install") => format!("{h} {install}"),
                Some(h) => h,
                None => install.to_string(),
            });
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid_and_says_what_it_is() {
        Cli::command().debug_assert();
        let help = Cli::command().render_help().to_string();
        assert!(help.contains("VeriModel \u{2014} IFC QA Gate + Quantity Takeoff + BCF Export"));
        for sub in ["check", "qto", "clash", "validate-config", "inspect", "doctor"] {
            assert!(help.contains(sub), "missing subcommand {sub}");
        }
    }

    #[test]
    fn parses_check_flags() {
        let cli = Cli::try_parse_from([
            "verimodel", "check", "--ifc", "a.ifc", "--ids", "b.ids", "--config", "p.yaml", "--out", "o", "--no-geometry", "--json",
        ])
        .unwrap();
        match cli.command {
            Command::Check(a) => {
                assert!(a.json && a.common.no_geometry);
                assert_eq!(a.common.out, PathBuf::from("o"));
                assert_eq!(a.ids, Some(PathBuf::from("b.ids")));
            }
            _ => panic!("wrong subcommand"),
        }
    }

    #[test]
    fn default_out_is_dot_out() {
        let cli = Cli::try_parse_from(["verimodel", "qto", "--ifc", "a.ifc"]).unwrap();
        match cli.command {
            Command::Qto(a) => assert_eq!(a.common.out, PathBuf::from("./out")),
            _ => panic!(),
        }
    }

    #[test]
    fn missing_ifc_is_a_clean_error() {
        let e = require_file(Path::new("/definitely/not/here.ifc"), "IFC file").unwrap_err();
        assert!(e.render().starts_with("Error: IFC file not found"));
        assert!(e.render().contains("Hint:"));
    }

    #[test]
    fn config_dir_is_parent_of_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("project.yaml");
        std::fs::write(&p, "project:\n  name: Demo\n").unwrap();
        let (cfg, d) = load_config(Some(&p)).unwrap();
        assert_eq!(cfg.project.name, "Demo");
        assert_eq!(d.unwrap(), dir.path());
    }

    #[test]
    fn invalid_config_gives_error_not_debug() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.yaml");
        std::fs::write(&p, "nonsense_key: 1\n").unwrap();
        let e = load_config(Some(&p)).unwrap_err();
        assert!(e.render().starts_with("Error: Configuration problem"));
    }

    #[test]
    fn clash_is_never_silent() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.ifc");
        std::fs::write(&a, "x").unwrap();
        let code = cmd_clash(ClashArgs { ifc_a: a.clone(), ifc_b: a, config: None, out: dir.path().join("o") }).unwrap();
        assert_eq!(code, EXIT_NOT_IMPLEMENTED);
    }

    #[test]
    fn validate_schema_and_file() {
        assert_eq!(cmd_validate(ValidateArgs { config: None, schema: true }).unwrap(), 0);
        assert!(cmd_validate(ValidateArgs { config: None, schema: false }).is_err());
    }
}
