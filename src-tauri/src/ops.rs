//! Plain, Tauri-free functions behind the commands, so they can be unit-tested
//! without launching a window.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use verimodel_core::config::Config;
use verimodel_core::model::{Model, ModelInfo, UnitSystem};
use verimodel_core::run::{self, RunInputs, RunOutcome};
use verimodel_ifc_reader::ReadOptions;

use crate::error::AppError;

// ---------------------------------------------------------------------------------------------
// File card
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardUnits {
    /// "metric" or "imperial".
    pub system: String,
    /// Length unit as declared in the file, e.g. "MILLIMETRE" (null when not declared).
    pub length_unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileCard {
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub sha256_short: String,
    pub schema: String,
    pub element_count: usize,
    pub units: CardUnits,
    pub disciplines: Vec<String>,
    pub storeys: Vec<String>,
    pub type_counts: BTreeMap<String, usize>,
    pub reader_warnings: Vec<String>,
}

/// Build the file card from what the reader found. Fills size and hash from the
/// file itself when the reader left them empty.
pub fn build_file_card(path: &Path, info: &ModelInfo) -> FileCard {
    let size_bytes = if info.file_size_bytes > 0 {
        info.file_size_bytes
    } else {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    };
    let sha256 = if info.sha256.is_empty() {
        verimodel_core::hash::sha256_file(path).unwrap_or_default()
    } else {
        info.sha256.clone()
    };
    let file_name = if info.file_name.is_empty() {
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        info.file_name.clone()
    };
    FileCard {
        file_name,
        path: path.display().to_string(),
        size_bytes,
        sha256_short: verimodel_core::hash::short(&sha256),
        sha256,
        schema: info.schema.clone(),
        element_count: info.element_count,
        units: CardUnits {
            system: match info.units.system {
                UnitSystem::Metric => "metric".into(),
                UnitSystem::Imperial => "imperial".into(),
            },
            length_unit: info.units.length_unit.clone(),
        },
        disciplines: info.disciplines.clone(),
        storeys: info.storeys.clone(),
        type_counts: info.type_counts.clone(),
        reader_warnings: info.reader_warnings.clone(),
    }
}

pub fn inspect_file(path: &str) -> Result<FileCard, AppError> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err(AppError::file_missing("IFC file", path));
    }
    let info = verimodel_ifc_reader::inspect(p)?;
    Ok(build_file_card(p, &info))
}

// ---------------------------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationResult {
    pub ok: bool,
    pub error: Option<String>,
}

pub fn default_config_yaml() -> String {
    let body = Config::default().to_yaml().unwrap_or_default();
    format!("# VeriModel project settings. Every key is optional; removed keys fall back to these defaults.\n{body}")
}

pub fn validate_config_yaml(yaml: &str) -> ValidationResult {
    match Config::from_yaml(yaml) {
        Ok(_) => ValidationResult {
            ok: true,
            error: None,
        },
        Err(e) => ValidationResult {
            ok: false,
            error: Some(e.to_string()),
        },
    }
}

/// Parse the config given by the UI, or the defaults when there is none (or it is blank).
pub fn parse_config(yaml: Option<&str>) -> Result<Config, AppError> {
    match yaml {
        Some(t) if !t.trim().is_empty() => Ok(Config::from_yaml(t)?),
        _ => Ok(Config::default()),
    }
}

// ---------------------------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------------------------

/// `<runs_root>/<ifc stem>-<yyyymmdd-hhmmss>`.
pub fn default_out_dir(runs_root: &Path, ifc_path: &Path, stamp: &str) -> PathBuf {
    let stem = ifc_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "model".into());
    let safe: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    runs_root.join(format!("{safe}-{stamp}"))
}

pub fn timestamp_now() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub struct RunRequest {
    pub ifc_path: PathBuf,
    pub ids_path: Option<PathBuf>,
    pub config_yaml: Option<String>,
    /// Folder relative paths inside the config resolve against (rates CSV, rule pack, IDS).
    pub config_dir: Option<PathBuf>,
    pub out_dir: PathBuf,
}

/// Read the model (geometry on), evaluate, write the out folder. Returns the outcome
/// and the model so the caller can keep it for the element detail panel.
pub fn run_check_blocking(req: &RunRequest) -> Result<(RunOutcome, Model), AppError> {
    let started = Instant::now();
    if !req.ifc_path.is_file() {
        return Err(AppError::file_missing(
            "IFC file",
            &req.ifc_path.display().to_string(),
        ));
    }
    let config = parse_config(req.config_yaml.as_deref())?;
    // An explicit IDS wins; otherwise use requirements.ids_file from the config.
    let ids_path = req
        .ids_path
        .clone()
        .or_else(|| config.ids_path(req.config_dir.as_deref()));
    if let Some(ids) = &ids_path {
        if !ids.is_file() {
            return Err(AppError::file_missing(
                "IDS file",
                &ids.display().to_string(),
            ));
        }
    }
    let opts = ReadOptions {
        ids_path: ids_path.clone(),
        geometry: true,
        ..Default::default()
    };
    log::info!("run_check: reading {}", req.ifc_path.display());
    let model = verimodel_ifc_reader::read_model(&req.ifc_path, &opts)?;
    let inputs = RunInputs {
        ifc_path: req.ifc_path.clone(),
        ids_path,
        config_path: None,
        config_dir: req.config_dir.clone(),
        out_dir: req.out_dir.clone(),
        started: Some(started),
    };
    let outcome = run::execute(&model, &config, &inputs)?;
    log::info!(
        "run_check: done verdict={:?} score={} out={}",
        outcome.report.summary.verdict,
        outcome.report.summary.score,
        outcome.out_dir.display()
    );
    Ok((outcome, model))
}

// ---------------------------------------------------------------------------------------------
// Examples
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Example {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ifc_path: String,
    /// `null` when the example project file is not present.
    pub config_path: Option<String>,
    /// `null` when the sample IDS is not present.
    pub ids_path: Option<String>,
}

/// Where the example project files and sample IFC files live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExampleRoots {
    /// Folder containing `residential/`, `office/`, `infrastructure/`, `sample.ids`.
    pub examples: PathBuf,
    /// Folder containing the `*.ifc` sample files.
    pub fixtures: PathBuf,
}

/// Pick the bundled resource folders when they exist (installed app), otherwise the
/// repository folders (development). `dev_root` is the repository root.
pub fn resolve_roots(bundled: Option<ExampleRoots>, dev_root: &Path) -> ExampleRoots {
    if let Some(b) = bundled {
        if b.examples.is_dir() && b.fixtures.is_dir() {
            return b;
        }
    }
    ExampleRoots {
        examples: dev_root.join("examples"),
        fixtures: dev_root.join("tests").join("fixtures"),
    }
}

struct ExampleDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    ifc: &'static str,
}

const EXAMPLE_DEFS: [ExampleDef; 3] = [
    ExampleDef {
        id: "residential",
        name: "Residential building",
        description: "Architecture model in IFC2X3 with an IDS file and a unit rates table: walls, doors, windows and slabs.",
        ifc: "bsi_ifc2x3_building_architecture.ifc",
    },
    ExampleDef {
        id: "office",
        name: "Office building",
        description: "Structural model in IFC4: columns, beams and slabs checked against load-bearing and classification requirements.",
        ifc: "bsi_ifc4_building_structural.ifc",
    },
    ExampleDef {
        id: "infrastructure",
        name: "Infrastructure (bridge)",
        description: "Bridge in IFC4X3: shows the checks on a non-building model with its own spatial structure.",
        ifc: "bsi_ifc4x3_infra_bridge.ifc",
    },
];

/// The demo pavilion: always the first example, because it is the fastest way to see VeriModel
/// at work (its `project.yaml` references `../sample.ids` and `../rates.csv`).
pub const DEMO_ID: &str = "demo";
const DEMO_NAME: &str = "Demo pavilion (start here)";
const DEMO_DESCRIPTION: &str = "Two-storey office pavilion with properties, quantities, classification and five deliberate defects \u{2014} the fastest way to see VeriModel at work.";

/// List the examples whose IFC file exists, the demo first. Missing optional files become `null`.
pub fn list_examples_in(roots: &ExampleRoots) -> Vec<Example> {
    let existing = |p: PathBuf| {
        if p.is_file() {
            Some(p.display().to_string())
        } else {
            None
        }
    };
    let ids = existing(roots.examples.join("sample.ids"));
    let demo_dir = roots.examples.join(DEMO_ID);
    let demo = existing(demo_dir.join("demo_building.ifc")).map(|ifc| Example {
        id: DEMO_ID.into(),
        name: DEMO_NAME.into(),
        description: DEMO_DESCRIPTION.into(),
        ifc_path: ifc,
        config_path: existing(demo_dir.join("project.yaml")),
        ids_path: ids.clone(),
    });
    demo.into_iter()
        .chain(EXAMPLE_DEFS.iter().filter_map(|d| {
            let ifc = existing(roots.fixtures.join(d.ifc))?;
            Some(Example {
                id: d.id.into(),
                name: d.name.into(),
                description: d.description.into(),
                ifc_path: ifc,
                config_path: existing(roots.examples.join(d.id).join("project.yaml")),
                ids_path: ids.clone(),
            })
        }))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------

pub fn save_text_file(path: &str, contents: &str) -> Result<(), AppError> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(p, contents)?;
    Ok(())
}

pub fn read_text_file(path: &str) -> Result<String, AppError> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err(AppError::file_missing("File", path));
    }
    Ok(std::fs::read_to_string(p)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use verimodel_core::model::ModelUnits;

    fn info() -> ModelInfo {
        let mut type_counts = BTreeMap::new();
        type_counts.insert("IfcWall".to_string(), 12);
        ModelInfo {
            file_name: "a.ifc".into(),
            file_size_bytes: 2048,
            sha256: "0123456789abcdef0123456789abcdef".into(),
            schema: "IFC4".into(),
            units: ModelUnits {
                length_unit: Some("MILLIMETRE".into()),
                ..Default::default()
            },
            element_count: 12,
            type_counts,
            storeys: vec!["Level 1".into()],
            disciplines: vec!["Architecture".into()],
            reader_warnings: vec!["a warning".into()],
            ..Default::default()
        }
    }

    #[test]
    fn file_card_copies_reader_facts() {
        let c = build_file_card(Path::new("/x/a.ifc"), &info());
        assert_eq!(c.file_name, "a.ifc");
        assert_eq!(c.path, "/x/a.ifc");
        assert_eq!(c.size_bytes, 2048);
        assert_eq!(c.sha256_short, "0123456789ab");
        assert_eq!(c.schema, "IFC4");
        assert_eq!(c.units.system, "metric");
        assert_eq!(c.units.length_unit.as_deref(), Some("MILLIMETRE"));
        assert_eq!(c.type_counts["IfcWall"], 12);
        assert_eq!(c.reader_warnings, vec!["a warning"]);
    }

    #[test]
    fn file_card_json_shape_is_the_documented_contract() {
        let v = serde_json::to_value(build_file_card(Path::new("/x/a.ifc"), &info())).unwrap();
        for k in [
            "file_name",
            "path",
            "size_bytes",
            "sha256",
            "sha256_short",
            "schema",
            "element_count",
            "units",
            "disciplines",
            "storeys",
            "type_counts",
            "reader_warnings",
        ] {
            assert!(v.get(k).is_some(), "missing key {k}");
        }
        assert!(v["units"].get("system").is_some() && v["units"].get("length_unit").is_some());
    }

    #[test]
    fn file_card_computes_hash_and_size_when_reader_left_them_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.ifc");
        std::fs::write(&p, b"abc").unwrap();
        let c = build_file_card(&p, &ModelInfo::default());
        assert_eq!(c.size_bytes, 3);
        assert_eq!(c.sha256_short, "ba7816bf8f01");
        assert_eq!(c.file_name, "t.ifc");
    }

    #[test]
    fn inspect_missing_file_is_a_plain_error() {
        let e = inspect_file("/no/such/file.ifc").unwrap_err();
        assert_eq!(e.code, "file_not_found");
        assert!(e.message.contains("/no/such/file.ifc"));
    }

    #[test]
    fn default_config_validates_and_roundtrips() {
        let y = default_config_yaml();
        assert!(y.starts_with('#'));
        assert_eq!(
            validate_config_yaml(&y),
            ValidationResult {
                ok: true,
                error: None
            }
        );
        assert!(parse_config(Some(&y)).is_ok());
    }

    #[test]
    fn invalid_config_reports_error_text() {
        let r = validate_config_yaml("nonsense_key: 1\n");
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("Configuration problem"));
        let r = validate_config_yaml("qto:\n  group_by: [colour]\n");
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("group_by"));
    }

    #[test]
    fn blank_or_missing_config_means_defaults() {
        assert_eq!(parse_config(None).unwrap(), Config::default());
        assert_eq!(parse_config(Some("  \n")).unwrap(), Config::default());
        assert_eq!(
            parse_config(Some("nope: 1")).unwrap_err().code,
            "config_invalid"
        );
    }

    #[test]
    fn out_dir_uses_stem_and_stamp_and_sanitises() {
        let d = default_out_dir(
            Path::new("/runs"),
            Path::new("/m/My Model (v2).ifc"),
            "20260101-120000",
        );
        assert_eq!(d, PathBuf::from("/runs/My_Model__v2_-20260101-120000"));
        let t = timestamp_now();
        assert_eq!(t.len(), 15);
        assert_eq!(&t[8..9], "-");
    }

    #[test]
    fn run_check_rejects_missing_inputs_before_reading() {
        let req = RunRequest {
            ifc_path: "/no/model.ifc".into(),
            ids_path: None,
            config_yaml: None,
            config_dir: None,
            out_dir: "/tmp/x".into(),
        };
        assert_eq!(
            run_check_blocking(&req).err().unwrap().code,
            "file_not_found"
        );
        let dir = tempfile::tempdir().unwrap();
        let ifc = dir.path().join("m.ifc");
        std::fs::write(&ifc, "x").unwrap();
        let req = RunRequest {
            ifc_path: ifc.clone(),
            ids_path: Some("/no/x.ids".into()),
            config_yaml: None,
            config_dir: None,
            out_dir: dir.path().join("o"),
        };
        let e = run_check_blocking(&req).err().unwrap();
        assert!(e.message.starts_with("IDS file not found"));
        let req = RunRequest {
            ifc_path: ifc,
            ids_path: None,
            config_yaml: Some("bad_key: 1".into()),
            config_dir: None,
            out_dir: dir.path().join("o"),
        };
        assert_eq!(
            run_check_blocking(&req).err().unwrap().code,
            "config_invalid"
        );
    }

    #[test]
    fn ids_file_from_config_is_resolved_against_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let ifc = dir.path().join("m.ifc");
        std::fs::write(&ifc, "x").unwrap();
        let req = RunRequest {
            ifc_path: ifc,
            ids_path: None,
            config_yaml: Some("requirements:\n  ids_file: ../missing.ids\n".into()),
            config_dir: Some(dir.path().join("cfg")),
            out_dir: dir.path().join("o"),
        };
        let e = run_check_blocking(&req).err().unwrap();
        assert_eq!(e.code, "file_not_found");
        assert!(
            e.message.contains("missing.ids") && e.message.contains("cfg"),
            "{}",
            e.message
        );
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }

    #[test]
    fn dev_roots_fall_back_to_repo_layout() {
        let repo = Path::new("/repo");
        let r = resolve_roots(None, repo);
        assert_eq!(r.examples, PathBuf::from("/repo/examples"));
        assert_eq!(r.fixtures, PathBuf::from("/repo/tests/fixtures"));
        let missing = ExampleRoots {
            examples: "/nope/a".into(),
            fixtures: "/nope/b".into(),
        };
        assert_eq!(resolve_roots(Some(missing), repo), r);
    }

    #[test]
    fn bundled_roots_win_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let b = ExampleRoots {
            examples: dir.path().join("examples"),
            fixtures: dir.path().join("fixtures"),
        };
        std::fs::create_dir_all(&b.examples).unwrap();
        std::fs::create_dir_all(&b.fixtures).unwrap();
        assert_eq!(resolve_roots(Some(b.clone()), Path::new("/repo")), b);
    }

    #[test]
    fn examples_resolve_paths_and_skip_missing_models() {
        let dir = tempfile::tempdir().unwrap();
        let roots = ExampleRoots {
            examples: dir.path().join("examples"),
            fixtures: dir.path().join("fixtures"),
        };
        touch(&roots.fixtures.join("bsi_ifc2x3_building_architecture.ifc"));
        touch(&roots.fixtures.join("bsi_ifc4_building_structural.ifc"));
        touch(&roots.examples.join("residential").join("project.yaml"));
        touch(&roots.examples.join("sample.ids"));
        let without_demo = list_examples_in(&roots);
        assert_eq!(
            without_demo
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["residential", "office"],
            "no demo files, no demo entry"
        );
        touch(&roots.examples.join("demo").join("demo_building.ifc"));
        touch(&roots.examples.join("demo").join("project.yaml"));
        let all = list_examples_in(&roots);
        assert_eq!(all[0].id, "demo");
        let list = all[1..].to_vec();
        assert_eq!(
            list.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["residential", "office"]
        );
        let res = &list[0];
        assert!(res
            .ifc_path
            .ends_with("bsi_ifc2x3_building_architecture.ifc"));
        assert!(res
            .config_path
            .as_ref()
            .unwrap()
            .ends_with("residential/project.yaml"));
        assert!(res.ids_path.as_ref().unwrap().ends_with("sample.ids"));
        assert!(
            list[1].config_path.is_none(),
            "office has no project.yaml in this fixture tree"
        );
    }

    #[test]
    fn demo_is_the_first_example_with_config_and_ids() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let list = list_examples_in(&resolve_roots(None, root));
        let demo = &list[0];
        assert_eq!(demo.id, "demo");
        assert_eq!(demo.name, "Demo pavilion (start here)");
        assert!(demo.description.contains("five deliberate defects"));
        assert!(
            demo.ifc_path.ends_with("examples/demo/demo_building.ifc")
                && Path::new(&demo.ifc_path).is_file()
        );
        assert!(demo
            .config_path
            .as_ref()
            .unwrap()
            .ends_with("examples/demo/project.yaml"));
        assert!(demo
            .ids_path
            .as_ref()
            .unwrap()
            .ends_with("examples/sample.ids"));
        assert!(
            list.len() >= 2 && list[1..].iter().all(|e| e.id != "demo"),
            "the buildingSMART samples follow the demo"
        );
    }

    #[test]
    fn repo_examples_resolve_in_dev_layout() {
        // CARGO_MANIFEST_DIR is src-tauri, so the repo root is its parent.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let roots = resolve_roots(None, root);
        let list = list_examples_in(&roots);
        assert!(!list.is_empty(), "fixtures exist in the repository");
        for e in &list {
            assert!(Path::new(&e.ifc_path).is_file());
        }
    }

    #[test]
    fn save_and_read_text_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("deep/er/x.yaml");
        save_text_file(&p.display().to_string(), "a: 1\n").unwrap();
        assert_eq!(read_text_file(&p.display().to_string()).unwrap(), "a: 1\n");
        assert_eq!(
            read_text_file("/no/such").unwrap_err().code,
            "file_not_found"
        );
    }
}
