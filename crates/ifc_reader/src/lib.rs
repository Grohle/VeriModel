//! # VeriModel IFC reader
//!
//! The ONLY crate that touches IFC files. It drives IfcOpenShell (Python) in a
//! subprocess through a bundled extractor script and turns its NDJSON stream
//! of facts into a [`verimodel_core::Model`]. It never decides pass/fail.
//!
//! ## Why a subprocess bridge
//! IfcOpenShell's Python API is the mature, complete surface (parsing for every
//! schema, geometry kernel, IDS via IfcTester, unit handling). Embedding a
//! Python interpreter would tie the binary to one Python build and make
//! installers fragile; a subprocess keeps VeriModel a plain Rust binary that
//! only needs `python3` with `ifcopenshell` (+ `ifctester`) on the machine.
//! The script is embedded in the binary and written to a temporary file at
//! run time, so there is nothing to install besides Python packages.
//!
//! ## Protocol
//! One JSON object per line (see `python/verimodel_extract.py`): `header`,
//! `spatial`, `element`*, `geometry`*, `ids`?, `warning`*, `end` | `error`.
//! Elements are streamed and consumed line by line.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use verimodel_core::model::{
    Classification, Element, Geometry, IdsResults, Model, ModelInfo, ModelUnits, PropValue, SpatialNode, SpatialRef,
    UnitSystem,
};

/// The bundled extractor script.
pub const EXTRACT_SCRIPT: &str = include_str!("../python/verimodel_extract.py");

/// Protocol version the Rust side understands.
pub const PROTOCOL_VERSION: u32 = 1;

/// Options for [`read_model`].
#[derive(Debug, Clone)]
pub struct ReadOptions {
    /// Optional IDS file to audit with IfcTester.
    pub ids_path: Option<PathBuf>,
    /// Run the geometry kernel (bounding boxes, volumes, areas). Slower, needed for geometry checks and QTO fallback.
    pub geometry: bool,
    /// Explicit Python interpreter; `None` = auto-detect (`VERIMODEL_PYTHON`, then `python3`, `python`, `py -3`).
    pub python: Option<PathBuf>,
    /// Threads for the geometry kernel; 0 = all cores.
    pub threads: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self { ids_path: None, geometry: true, python: None, threads: 0 }
    }
}

/// Plain-language reader error. `message` and `hint` are shown to the user as-is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReaderError {
    pub code: String,
    pub message: String,
    pub hint: String,
}

impl ReaderError {
    fn new(code: &str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), hint: hint.into() }
    }
}

impl std::fmt::Display for ReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if !self.hint.is_empty() {
            write!(f, " ({})", self.hint)?;
        }
        Ok(())
    }
}

impl std::error::Error for ReaderError {}

/// What `check_environment` found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EnvironmentInfo {
    /// Interpreter path that works.
    pub python: String,
    pub python_version: String,
    pub ifcopenshell_version: String,
    pub ifctester: bool,
}

const INSTALL_HINT: &str = "Install Python 3.9+ and run: pip install ifcopenshell ifctester  (or set VERIMODEL_PYTHON to an interpreter that has them)";

// ---------------------------------------------------------------------------
// Python discovery
// ---------------------------------------------------------------------------

/// Candidate interpreters, in order.
fn python_candidates(explicit: Option<&Path>) -> Vec<(String, Vec<String>)> {
    let mut v = Vec::new();
    if let Some(p) = explicit {
        v.push((p.display().to_string(), vec![]));
    }
    if let Ok(p) = std::env::var("VERIMODEL_PYTHON") {
        if !p.trim().is_empty() {
            v.push((p, vec![]));
        }
    }
    v.push(("python3".into(), vec![]));
    v.push(("python".into(), vec![]));
    if cfg!(windows) {
        v.push(("py".into(), vec!["-3".into()]));
    }
    v
}

/// Write the embedded script to a temp file (cached per process).
fn script_path() -> Result<PathBuf, ReaderError> {
    let dir = std::env::temp_dir().join(format!("verimodel-reader-{}", verimodel_core::VERSION));
    std::fs::create_dir_all(&dir).map_err(|e| ReaderError::new("io", format!("cannot create temp dir {}: {e}", dir.display()), ""))?;
    let path = dir.join("verimodel_extract.py");
    let needs_write = match std::fs::read_to_string(&path) {
        Ok(existing) => existing != EXTRACT_SCRIPT,
        Err(_) => true,
    };
    if needs_write {
        std::fs::write(&path, EXTRACT_SCRIPT).map_err(|e| ReaderError::new("io", format!("cannot write {}: {e}", path.display()), ""))?;
    }
    Ok(path)
}

#[derive(Deserialize)]
struct EnvLine {
    #[serde(default)]
    python: Option<String>,
    #[serde(default)]
    ifcopenshell_version: Option<String>,
    #[serde(default)]
    ifctester: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    protocol: u32,
}

/// Find a Python with IfcOpenShell. Cheap enough to call before every run.
pub fn check_environment_with(explicit: Option<&Path>) -> Result<EnvironmentInfo, ReaderError> {
    let script = script_path()?;
    let mut last_err = String::new();
    for (exe, pre) in python_candidates(explicit) {
        let out = Command::new(&exe).args(&pre).arg("-I").arg(&script).arg("--check-env").stdin(Stdio::null()).output();
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                last_err = format!("{exe}: {e}");
                continue;
            }
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let Some(line) = text.lines().find(|l| l.contains("\"kind\":\"env\"")) else {
            last_err = format!("{exe}: unexpected output: {}", String::from_utf8_lossy(&out.stderr).trim());
            continue;
        };
        let env: EnvLine = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                last_err = format!("{exe}: {e}");
                continue;
            }
        };
        if env.protocol != PROTOCOL_VERSION {
            last_err = format!("{exe}: extractor protocol {} != {}", env.protocol, PROTOCOL_VERSION);
            continue;
        }
        match (env.ifcopenshell_version, env.error) {
            (Some(v), None) => {
                return Ok(EnvironmentInfo {
                    python: exe,
                    python_version: env.python.unwrap_or_default(),
                    ifcopenshell_version: v,
                    ifctester: env.ifctester,
                })
            }
            (_, Some(e)) => last_err = format!("{exe}: {e}"),
            _ => last_err = format!("{exe}: IfcOpenShell not importable"),
        }
    }
    Err(ReaderError::new(
        "python_missing",
        format!("No Python interpreter with IfcOpenShell was found ({last_err})."),
        INSTALL_HINT,
    ))
}

/// Auto-detect the environment.
pub fn check_environment() -> Result<EnvironmentInfo, ReaderError> {
    check_environment_with(None)
}

// ---------------------------------------------------------------------------
// NDJSON line types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct HeaderLine {
    #[serde(default)]
    protocol: u32,
    schema: String,
    #[serde(default)]
    schema_raw: Option<String>,
    #[serde(default)]
    mvd: Option<String>,
    #[serde(default)]
    authoring_tool: Option<String>,
    #[serde(default)]
    project_name: Option<String>,
    #[serde(default)]
    units: UnitsLine,
    #[serde(default)]
    file_size_bytes: u64,
    #[serde(default)]
    geometry_available: bool,
}

#[derive(Deserialize, Default)]
struct UnitsLine {
    #[serde(default)]
    length_unit: Option<String>,
    #[serde(default = "one")]
    length_to_m: f64,
    #[serde(default)]
    area_unit: Option<String>,
    #[serde(default = "one")]
    area_to_m2: f64,
    #[serde(default)]
    volume_unit: Option<String>,
    #[serde(default = "one")]
    volume_to_m3: f64,
    #[serde(default)]
    system: Option<String>,
}

fn one() -> f64 {
    1.0
}

#[derive(Deserialize)]
struct SpatialLine {
    #[serde(default)]
    tree: Vec<SpatialNode>,
}

#[derive(Deserialize)]
struct ElementLine {
    #[serde(default)]
    global_id: String,
    #[serde(default)]
    step_id: i64,
    ifc_type: String,
    #[serde(default)]
    ancestors: Vec<String>,
    #[serde(default)]
    predefined_type: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    object_type: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    type_name: Option<String>,
    #[serde(default)]
    storey: Option<String>,
    #[serde(default)]
    container: Option<SpatialRef>,
    #[serde(default)]
    psets: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    quantities: BTreeMap<String, BTreeMap<String, f64>>,
    #[serde(default)]
    classifications: Vec<Classification>,
    #[serde(default)]
    materials: Vec<String>,
    #[serde(default)]
    has_representation: bool,
}

#[derive(Deserialize)]
struct GeometryLine {
    #[serde(default)]
    global_id: String,
    /// STEP id: geometry is merged by step id, never by GlobalId, because
    /// duplicated GlobalIds are exactly one of the defects we must detect.
    #[serde(default)]
    step_id: i64,
    bbox_min: [f64; 3],
    bbox_max: [f64; 3],
    #[serde(default)]
    volume_m3: Option<f64>,
    #[serde(default)]
    area_m2: Option<f64>,
    #[serde(default)]
    triangle_count: u64,
}

#[derive(Deserialize)]
struct WarningLine {
    message: String,
}

#[derive(Deserialize)]
struct ErrorLine {
    code: String,
    message: String,
    #[serde(default)]
    hint: String,
}

#[derive(Deserialize)]
struct KindOnly {
    kind: String,
}

fn json_to_prop(v: serde_json::Value) -> PropValue {
    match v {
        serde_json::Value::Null => PropValue::Null,
        serde_json::Value::Bool(b) => PropValue::Bool(b),
        serde_json::Value::Number(n) => PropValue::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => PropValue::Text(s),
        serde_json::Value::Array(a) => PropValue::List(a.into_iter().map(json_to_prop).collect()),
        serde_json::Value::Object(o) => PropValue::Text(serde_json::Value::Object(o).to_string()),
    }
}

impl From<ElementLine> for Element {
    fn from(l: ElementLine) -> Self {
        Element {
            global_id: l.global_id,
            step_id: l.step_id,
            ifc_type: l.ifc_type,
            ancestors: l.ancestors,
            predefined_type: l.predefined_type,
            name: l.name,
            description: l.description,
            object_type: l.object_type,
            tag: l.tag,
            type_name: l.type_name,
            storey: l.storey,
            container: l.container,
            psets: l
                .psets
                .into_iter()
                .map(|(k, props)| (k, props.into_iter().map(|(pk, pv)| (pk, json_to_prop(pv))).collect()))
                .collect(),
            quantities: l.quantities,
            classifications: l.classifications,
            materials: l.materials,
            has_representation: l.has_representation,
            geometry: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

fn spawn_extractor(ifc_path: &Path, opts: &ReadOptions, mode: &str) -> Result<(std::process::Child, String), ReaderError> {
    if !ifc_path.is_file() {
        return Err(ReaderError::new(
            "not_found",
            format!("The file '{}' does not exist.", ifc_path.display()),
            "Check the path and that the file was not moved.",
        ));
    }
    let env = check_environment_with(opts.python.as_deref())?;
    let script = script_path()?;
    let mut cmd = Command::new(&env.python);
    if env.python == "py" {
        cmd.arg("-3");
    }
    cmd.arg("-I").arg(&script).arg("--ifc").arg(ifc_path).arg("--mode").arg(mode);
    if let Some(ids) = &opts.ids_path {
        cmd.arg("--ids").arg(ids);
    }
    if !opts.geometry {
        cmd.arg("--no-geometry");
    }
    if opts.threads > 0 {
        cmd.arg("--threads").arg(opts.threads.to_string());
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| ReaderError::new("spawn_failed", format!("Cannot start the IFC reader ({}): {e}", env.python), INSTALL_HINT))?;
    Ok((child, env.python))
}

/// Read a model fully (elements, geometry, IDS).
pub fn read_model(ifc_path: &Path, opts: &ReadOptions) -> Result<Model, ReaderError> {
    read_with_mode(ifc_path, opts, "full")
}

/// Quick inspection for the file card: no geometry, no IDS.
pub fn inspect(ifc_path: &Path) -> Result<ModelInfo, ReaderError> {
    let opts = ReadOptions { geometry: false, ..Default::default() };
    Ok(read_with_mode(ifc_path, &opts, "inspect")?.info)
}

fn read_with_mode(ifc_path: &Path, opts: &ReadOptions, mode: &str) -> Result<Model, ReaderError> {
    if !ifc_path.is_file() {
        return Err(ReaderError::new(
            "not_found",
            format!("The file '{}' does not exist.", ifc_path.display()),
            "Check the path and that the file was not moved.",
        ));
    }
    let sha = verimodel_core::hash::sha256_file(ifc_path)
        .map_err(|e| ReaderError::new("io", format!("Cannot read '{}': {e}", ifc_path.display()), ""))?;
    let (mut child, _python) = spawn_extractor(ifc_path, opts, mode)?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stderr_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut s);
        s
    });

    let mut model = Model::default();
    model.info.file_name = ifc_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    model.info.sha256 = sha;
    let mut geometry: BTreeMap<i64, Geometry> = BTreeMap::new();
    let mut got_header = false;
    let mut got_end = false;
    let mut geometry_lines = 0usize;

    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| ReaderError::new("io", format!("Reader output could not be read: {e}"), ""))?;
        if line.trim().is_empty() {
            continue;
        }
        let kind: KindOnly = match serde_json::from_str(&line) {
            Ok(k) => k,
            Err(_) => {
                model.info.reader_warnings.push(format!("Unreadable line from the IFC reader: {}", truncate(&line, 120)));
                continue;
            }
        };
        match kind.kind.as_str() {
            "header" => {
                let h: HeaderLine = parse(&line)?;
                if h.protocol != PROTOCOL_VERSION {
                    return Err(ReaderError::new("protocol", format!("IFC reader protocol {} is not supported (expected {}).", h.protocol, PROTOCOL_VERSION), "Reinstall VeriModel."));
                }
                model.info.schema = h.schema;
                model.info.mvd = h.mvd;
                model.info.authoring_tool = h.authoring_tool;
                model.info.project_name = h.project_name;
                model.info.file_size_bytes = h.file_size_bytes;
                model.info.geometry_available = h.geometry_available;
                model.info.units = ModelUnits {
                    length_unit: h.units.length_unit,
                    length_to_m: h.units.length_to_m,
                    area_unit: h.units.area_unit,
                    area_to_m2: h.units.area_to_m2,
                    volume_unit: h.units.volume_unit,
                    volume_to_m3: h.units.volume_to_m3,
                    system: if h.units.system.as_deref() == Some("imperial") { UnitSystem::Imperial } else { UnitSystem::Metric },
                };
                if let Some(raw) = h.schema_raw {
                    if raw != model.info.schema {
                        log::debug!("schema {raw} normalised to {}", model.info.schema);
                    }
                }
                got_header = true;
            }
            "spatial" => {
                let s: SpatialLine = parse(&line)?;
                model.info.spatial_tree = s.tree;
            }
            "element" => {
                let e: ElementLine = parse(&line)?;
                model.elements.push(e.into());
            }
            "geometry" => {
                let g: GeometryLine = parse(&line)?;
                geometry_lines += 1;
                let _ = &g.global_id;
                geometry.insert(
                    g.step_id,
                    Geometry { bbox_min: g.bbox_min, bbox_max: g.bbox_max, volume_m3: g.volume_m3, area_m2: g.area_m2, triangle_count: g.triangle_count },
                );
            }
            "geometry_unavailable" => model.info.geometry_available = false,
            "ids" => {
                let r: IdsResults = parse(&line)?;
                model.ids_results = Some(r);
            }
            "warning" => {
                let w: WarningLine = parse(&line)?;
                model.info.reader_warnings.push(w.message);
            }
            "end" => got_end = true,
            "error" => {
                let e: ErrorLine = parse(&line)?;
                let _ = child.wait();
                return Err(ReaderError { code: e.code, message: e.message, hint: e.hint });
            }
            other => model.info.reader_warnings.push(format!("Unknown reader message '{other}' ignored")),
        }
    }
    let status = child.wait().map_err(|e| ReaderError::new("io", format!("IFC reader process failed: {e}"), ""))?;
    let stderr_text = stderr_thread.join().unwrap_or_default();
    if !got_header {
        return Err(ReaderError::new(
            "no_output",
            "The IFC reader produced no output.".to_string(),
            format!("{} Details: {}", INSTALL_HINT, truncate(stderr_text.trim(), 400)),
        ));
    }
    if !got_end || !status.success() {
        return Err(ReaderError::new(
            "reader_crashed",
            "The IFC reader stopped before finishing the file.".to_string(),
            format!("The file may be corrupt or too large for the available memory. Details: {}", truncate(stderr_text.trim(), 400)),
        ));
    }
    if !stderr_text.trim().is_empty() {
        log::debug!("reader stderr: {}", stderr_text.trim());
    }

    // Attach geometry, derive counts, disciplines, storeys.
    for e in &mut model.elements {
        if let Some(g) = geometry.remove(&e.step_id) {
            e.geometry = Some(g);
        }
    }
    if model.info.geometry_available && geometry_lines == 0 && model.elements.iter().any(|e| e.has_representation) && mode == "full" {
        model.info.reader_warnings.push("The geometry kernel produced no shapes; geometry checks were skipped.".into());
        model.info.geometry_available = false;
    }
    model.sort_elements();
    model.info.element_count = model.elements.len();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &model.elements {
        *counts.entry(e.ifc_type.clone()).or_default() += 1;
    }
    model.info.type_counts = counts;
    model.info.disciplines = verimodel_core::disciplines::detect(&model.elements);
    model.info.storeys = storeys_in_order(&model.info.spatial_tree);
    Ok(model)
}

fn parse<'a, T: Deserialize<'a>>(line: &'a str) -> Result<T, ReaderError> {
    serde_json::from_str(line).map_err(|e| ReaderError::new("protocol", format!("Unexpected data from the IFC reader: {e}"), "Reinstall VeriModel or report this as a bug."))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// Storey names in tree order (elevation order within a building).
fn storeys_in_order(tree: &[SpatialNode]) -> Vec<String> {
    let mut out = Vec::new();
    for root in tree {
        for n in root.walk() {
            if n.ifc_type.eq_ignore_ascii_case("IfcBuildingStorey") {
                let name = n.name.clone().unwrap_or_else(|| n.global_id.clone());
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_line_maps_into_element() {
        let line = r#"{"kind":"element","global_id":"0fqX614OH1YO1Njdxms2$Q","step_id":5,"ifc_type":"IfcWall","ancestors":["IfcWall","IfcElement"],"name":"W","psets":{"Pset_WallCommon":{"FireRating":"REI60","IsExternal":true,"X":null,"N":2.5}},"quantities":{"Qto_WallBaseQuantities":{"NetVolume":1.5}},"has_representation":true}"#;
        let e: Element = serde_json::from_str::<ElementLine>(line).unwrap().into();
        assert_eq!(e.ifc_type, "IfcWall");
        assert_eq!(e.psets["Pset_WallCommon"]["FireRating"], PropValue::Text("REI60".into()));
        assert_eq!(e.psets["Pset_WallCommon"]["IsExternal"], PropValue::Bool(true));
        assert_eq!(e.psets["Pset_WallCommon"]["X"], PropValue::Null);
        assert_eq!(e.quantity("NetVolume"), Some(1.5));
        assert!(e.is_a("IfcElement"));
    }

    #[test]
    fn script_is_embedded_and_cached() {
        assert!(EXTRACT_SCRIPT.contains("PROTOCOL_VERSION = 1"));
        let p = script_path().unwrap();
        assert!(p.is_file());
    }

    #[test]
    fn missing_file_is_a_plain_error() {
        let err = read_model(Path::new("/definitely/not/here.ifc"), &ReadOptions::default()).unwrap_err();
        assert_eq!(err.code, "not_found");
        assert!(err.message.contains("does not exist"));
    }
}
