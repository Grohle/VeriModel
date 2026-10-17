//! Helpers shared by the end-to-end tests.

use std::path::{Path, PathBuf};

use verimodel_core::config::Config;
use verimodel_core::model::Model;
use verimodel_core::run::{execute, RunInputs, RunOutcome};
use verimodel_ifc_reader::{read_model, ReadOptions};

/// Absolute path of a file under `tests/fixtures/`.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
}

/// Absolute path of a file under `examples/`.
pub fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples").join(name)
}

/// Whether the IFC-dependent tests are explicitly skipped (never silently).
pub fn skip_ifc() -> bool {
    if std::env::var("VERIMODEL_SKIP_IFC_TESTS").is_ok() {
        eprintln!("VERIMODEL_SKIP_IFC_TESTS set: skipping");
        return true;
    }
    false
}

/// Read a fixture with geometry and an optional IDS.
pub fn read(name: &str, ids: Option<&Path>) -> Model {
    let opts = ReadOptions { ids_path: ids.map(|p| p.to_path_buf()), ..Default::default() };
    read_model(&fixture(name), &opts).unwrap_or_else(|e| panic!("reading {name}: {e}"))
}

/// Run the whole pipeline into `out_dir`.
pub fn run(model: &Model, config: &Config, config_dir: Option<&Path>, ids: Option<&Path>, out_dir: &Path) -> RunOutcome {
    let inputs = RunInputs {
        ifc_path: fixture(&model.info.file_name),
        ids_path: ids.map(|p| p.to_path_buf()),
        config_path: None,
        config_dir: config_dir.map(|p| p.to_path_buf()),
        out_dir: out_dir.to_path_buf(),
        started: None,
    };
    execute(model, config, &inputs).expect("run")
}

/// Names of the topic folders inside a .bcfzip (GUID directories).
pub fn bcf_topics(path: &Path) -> Vec<String> {
    let f = std::fs::File::open(path).expect("bcfzip");
    let mut z = zip::ZipArchive::new(f).expect("zip");
    let mut topics = std::collections::BTreeSet::new();
    for i in 0..z.len() {
        let name = z.by_index(i).unwrap().name().to_string();
        if name.ends_with("markup.bcf") {
            if let Some(dir) = name.split('/').next() {
                topics.insert(dir.to_string());
            }
        }
    }
    topics.into_iter().collect()
}
