//! BCF writer (BIM Collaboration Format) and the matching `issues.csv`.
//!
//! * [`write_bcfzip`] writes a BCF 2.1 (`output.bcf_version == "2.1"`, the
//!   default) or BCF 3.0 (`"3.0"`) archive following the buildingSMART
//!   BCF-XML schemas (`release_2_1` / `release_3_0`):
//!   - `bcf.version` at the root (identical in content to the official file),
//!   - BCF 3.0 only: `extensions.xml` (required by 3.0) listing every topic
//!     type, status, priority, label and user used in the archive,
//!   - one folder per topic named after the lowercase topic GUID, holding
//!     `markup.bcf` and, when there is something to show, `viewpoint.bcfv`.
//!
//!   One topic is written per ERROR or WARNING finding (INFO is skipped), in
//!   the order of `report.findings` (already sorted by the engine).
//!   No snapshots are written: VeriModel has no renderer, and a viewpoint
//!   never references a snapshot file that does not exist.
//! * [`write_issues_csv`] writes one RFC 4180 row per BCF topic, with the
//!   same topic GUIDs, titles and descriptions as the archive.
//!
//! Determinism: topic GUIDs are UUID v5 values derived from the IFC SHA-256,
//! rule id, GlobalId and message, so re-running on the same file yields the
//! same GUIDs. Zip entries use a fixed timestamp, fixed permissions and a
//! fixed order, so two archives of the same report differ only in the
//! `CreationDate` of the topics (the real UTC time of the export).
//!
//! Camera: when the related element has a bounding box, the viewpoint holds
//! a perspective camera looking at the box centre from the south-west and
//! above, far enough away for the whole box to fit in a 60 degree field of
//! view. Without element geometry, BCF 2.1 viewpoints carry only the
//! component selection (the camera is optional in 2.1). BCF 3.0 requires a
//! camera in every viewpoint, so 3.0 falls back to framing the bounding box
//! of the whole model, and writes no viewpoint when the model has no
//! geometry at all.

use std::collections::{BTreeSet, HashMap};
use std::io::{Cursor, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::config::{Config, Language};
use crate::error::{CoreError, CoreResult};
use crate::i18n::Strings;
use crate::model::{
    format_number, Element, Finding, Geometry, Model, RuleResult, RunReport, Severity,
};

/// Numbers reported after writing a BCF file.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BcfStats {
    /// Number of topics written (one per ERROR/WARNING finding).
    pub topics: usize,
    /// Number of topics that carry a `viewpoint.bcfv` file.
    pub with_viewpoint: usize,
    /// BCF version actually written ("2.1" or "3.0").
    pub version: String,
}

/// Write `issues.bcfzip`.
pub fn write_bcfzip(
    report: &RunReport,
    model: &Model,
    config: &Config,
    out_path: &Path,
) -> CoreResult<BcfStats> {
    let (bytes, stats) = build_bcfzip(report, model, config, Utc::now())?;
    std::fs::write(out_path, bytes)
        .map_err(|e| CoreError::Io(format!("cannot write {}: {e}", out_path.display())))?;
    Ok(stats)
}

/// Write `issues.csv` (one row per ERROR/WARNING finding, i.e. one row per BCF topic).
///
/// Columns: severity, rule_id, topic_guid, global_id, ifc_type, name, storey,
/// title, description, expected, actual, fix_hint. The language of title and
/// description is `output.language` of the configuration embedded in the report.
pub fn write_issues_csv(report: &RunReport, out_path: &Path) -> CoreResult<usize> {
    let strings = Strings::for_language(report_language(report));
    let issues = collect_issues(report, &strings);
    let mut wtr = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(Vec::new());
    let csv_err = |e: csv::Error| CoreError::Report(format!("cannot write issues.csv: {e}"));
    wtr.write_record([
        "severity",
        "rule_id",
        "topic_guid",
        "global_id",
        "ifc_type",
        "name",
        "storey",
        "title",
        "description",
        "expected",
        "actual",
        "fix_hint",
    ])
    .map_err(csv_err)?;
    for issue in &issues {
        let f = issue.finding;
        let opt = |o: &Option<String>| o.clone().unwrap_or_default();
        wtr.write_record([
            f.severity.as_str().to_string(),
            f.rule_id.clone(),
            issue.guid.to_string(),
            opt(&f.global_id),
            opt(&f.ifc_type),
            opt(&f.name),
            opt(&f.storey),
            issue.title.clone(),
            issue.description.clone(),
            opt(&f.expected),
            opt(&f.actual),
            opt(&f.fix_hint),
        ])
        .map_err(csv_err)?;
    }
    let bytes = wtr
        .into_inner()
        .map_err(|e| CoreError::Report(format!("cannot write issues.csv: {e}")))?;
    std::fs::write(out_path, bytes)
        .map_err(|e| CoreError::Io(format!("cannot write {}: {e}", out_path.display())))?;
    Ok(issues.len())
}

// ---------------------------------------------------------------------------
// Version handling
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BcfVersion {
    V21,
    V30,
}

impl BcfVersion {
    /// Parse `output.bcf_version`. Unknown values fall back to 2.1 with a warning.
    fn from_config(value: &str) -> BcfVersion {
        match value.trim() {
            "2.1" | "2.1.0" | "21" => BcfVersion::V21,
            "3.0" | "3" | "3.0.0" | "30" => BcfVersion::V30,
            other => {
                log::warn!("output.bcf_version '{other}' is not supported (use \"2.1\" or \"3.0\"); writing BCF 2.1");
                BcfVersion::V21
            }
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            BcfVersion::V21 => "2.1",
            BcfVersion::V30 => "3.0",
        }
    }
}

// ---------------------------------------------------------------------------
// Issues: the shared, language-resolved view of the findings exported as topics
// ---------------------------------------------------------------------------

/// One BCF topic to write (also one row of `issues.csv`).
struct Issue<'a> {
    finding: &'a Finding,
    guid: Uuid,
    title: String,
    description: String,
    priority: String,
    labels: Vec<String>,
}

/// Placeholder for unknown values in titles and descriptions.
const NONE: &str = "-";
/// Titles longer than this (in characters) are shortened; the description keeps the full text.
const MAX_TITLE_CHARS: usize = 250;

/// Namespace of the deterministic topic GUIDs.
fn topic_namespace() -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, b"urn:verimodel:bcf:topic")
}

/// Deterministic topic GUID: UUID v5 of (IFC SHA-256, rule id, GlobalId, message).
/// `occurrence` > 0 disambiguates identical findings (e.g. duplicated GlobalIds).
fn topic_guid(ifc_sha256: &str, finding: &Finding, occurrence: usize) -> Uuid {
    let mut name = format!(
        "{}\n{}\n{}\n{}",
        ifc_sha256,
        finding.rule_id,
        finding.global_id.as_deref().unwrap_or(""),
        finding.message
    );
    if occurrence > 0 {
        name.push_str(&format!("\n#{occurrence}"));
    }
    Uuid::new_v5(&topic_namespace(), name.as_bytes())
}

/// Viewpoint GUID, derived from the topic GUID.
fn viewpoint_guid(topic: &Uuid) -> Uuid {
    Uuid::new_v5(topic, b"viewpoint")
}

/// Language recorded in the report's effective configuration (`output.language`).
fn report_language(report: &RunReport) -> Language {
    report
        .config
        .get("output")
        .and_then(|o| o.get("language"))
        .and_then(|l| serde_json::from_value::<Language>(l.clone()).ok())
        .unwrap_or_default()
}

/// Substitute `{key}` placeholders in one pass (values are never re-scanned,
/// so a message containing `{name}` stays intact).
fn fill_template(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 64);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match args.iter().find(|(k, _)| *k == key) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push('{');
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    s.push('…');
    s
}

/// Findings exported as topics, in report order, with their GUIDs and texts.
fn collect_issues<'a>(report: &'a RunReport, strings: &Strings) -> Vec<Issue<'a>> {
    let rules: HashMap<&str, &RuleResult> = report
        .rules
        .iter()
        .map(|r| (r.rule_id.as_str(), r))
        .collect();
    let generated_by = format!(
        "{} {} {}",
        strings.get("report.generated_by"),
        report.tool.name,
        report.tool.version
    );
    let mut seen: HashMap<Uuid, usize> = HashMap::new();
    let mut out = Vec::new();
    for f in report
        .findings
        .iter()
        .filter(|f| f.severity != Severity::Info)
    {
        let base = topic_guid(&report.model.sha256, f, 0);
        let n = seen.entry(base).or_insert(0);
        let guid = if *n == 0 {
            base
        } else {
            topic_guid(&report.model.sha256, f, *n)
        };
        *n += 1;

        let rule = rules.get(f.rule_id.as_str()).copied();
        let rule_description = rule
            .map(|r| r.description.as_str())
            .filter(|d| !d.trim().is_empty())
            .unwrap_or(&f.message);
        let or_none = |o: &Option<String>| {
            o.as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(NONE)
                .to_string()
        };

        let title = shorten(
            &fill_template(
                &strings.get("bcf.title"),
                &[("rule", &f.rule_id), ("message", &f.message)],
            ),
            MAX_TITLE_CHARS,
        );
        let body = fill_template(
            &strings.get("bcf.description"),
            &[
                ("rule", &f.rule_id),
                ("description", rule_description),
                ("message", &f.message),
                ("type", &or_none(&f.ifc_type)),
                ("name", &or_none(&f.name)),
                ("global_id", &or_none(&f.global_id)),
                ("storey", &or_none(&f.storey)),
                ("expected", &or_none(&f.expected)),
                ("actual", &or_none(&f.actual)),
                ("fix_hint", &or_none(&f.fix_hint)),
            ],
        );
        let description = format!("{body}\n\n{generated_by}");
        let priority = strings.get(&format!("bcf.priority.{}", f.severity.as_str()));

        let mut labels: Vec<String> = Vec::new();
        for l in [
            Some(report.tool.name.as_str()),
            rule.map(|r| r.category.as_str()),
            Some(f.rule_id.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            let l = l.trim();
            if !l.is_empty() && !labels.iter().any(|x| x == l) {
                labels.push(l.to_string());
            }
        }
        out.push(Issue {
            finding: f,
            guid,
            title,
            description,
            priority,
            labels,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Geometry -> camera
// ---------------------------------------------------------------------------

/// Field of view written in viewpoints (degrees). BCF 2.1 restricts it to 45..=60.
const FIELD_OF_VIEW_DEG: f64 = 60.0;
/// Aspect ratio written in BCF 3.0 viewpoints (required there). With 1.0 the
/// horizontal (2.1) and vertical (3.0) field of view are the same angle.
const ASPECT_RATIO: f64 = 1.0;
/// Minimum radius framed around an element, so tiny or flat boxes stay readable (metres).
const MIN_FRAME_RADIUS_M: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Camera {
    position: [f64; 3],
    direction: [f64; 3],
    up: [f64; 3],
}

fn is_valid_bbox(g: &Geometry) -> bool {
    g.bbox_min
        .iter()
        .chain(g.bbox_max.iter())
        .all(|v| v.is_finite())
        && (0..3).all(|i| g.bbox_max[i] >= g.bbox_min[i])
}

/// Perspective camera framing a bounding box: looks at the centre from the
/// south-west and above (direction (1, 1, -1)), at the distance where the
/// box's bounding sphere fits in the field of view.
fn camera_for_bbox(min: [f64; 3], max: [f64; 3]) -> Option<Camera> {
    let g = Geometry {
        bbox_min: min,
        bbox_max: max,
        ..Default::default()
    };
    if !is_valid_bbox(&g) {
        return None;
    }
    let c = g.centre();
    let e = g.extent();
    let radius = (0.5 * (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt()).max(MIN_FRAME_RADIUS_M);
    let half_fov = (FIELD_OF_VIEW_DEG / 2.0).to_radians();
    let distance = radius / half_fov.sin() * 1.1;
    let k = 1.0 / 3f64.sqrt();
    let direction = [k, k, -k];
    let position = [
        c[0] - direction[0] * distance,
        c[1] - direction[1] * distance,
        c[2] - direction[2] * distance,
    ];
    // World Z projected onto the view plane: (0,0,1) - (z.d) d, normalised = (1,1,2)/sqrt(6).
    let u = 1.0 / 6f64.sqrt();
    let up = [u, u, 2.0 * u];
    if position.iter().all(|v| v.is_finite()) {
        Some(Camera {
            position,
            direction,
            up,
        })
    } else {
        None
    }
}

/// Union of all element bounding boxes (used as the BCF 3.0 fallback camera).
fn model_bbox(model: &Model) -> Option<([f64; 3], [f64; 3])> {
    let mut acc: Option<([f64; 3], [f64; 3])> = None;
    for g in model
        .elements
        .iter()
        .filter_map(|e| e.geometry.as_ref())
        .filter(|g| is_valid_bbox(g))
    {
        acc = Some(match acc {
            None => (g.bbox_min, g.bbox_max),
            Some((mn, mx)) => (
                [
                    mn[0].min(g.bbox_min[0]),
                    mn[1].min(g.bbox_min[1]),
                    mn[2].min(g.bbox_min[2]),
                ],
                [
                    mx[0].max(g.bbox_max[0]),
                    mx[1].max(g.bbox_max[1]),
                    mx[2].max(g.bbox_max[2]),
                ],
            ),
        });
    }
    acc
}

/// True when `id` is a syntactically valid IfcGuid (22 chars of the IFC base64 alphabet),
/// the only form the BCF schemas accept in `IfcGuid` attributes.
fn is_ifc_guid(id: &str) -> bool {
    id.len() == 22
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
}

// ---------------------------------------------------------------------------
// Minimal XML writer (escaped, indented, deterministic)
// ---------------------------------------------------------------------------

/// Drop characters that XML 1.0 does not allow (control characters other than tab/newline/CR).
fn xml_clean(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().filter(|&c| {
        matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{FFFE}' && c != '\u{FFFF}')
    })
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in xml_clean(s) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in xml_clean(s) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out
}

struct Xml {
    out: String,
    depth: usize,
}

impl Xml {
    fn new() -> Self {
        Xml {
            out: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n"),
            depth: 0,
        }
    }

    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
    }

    fn tag(&mut self, name: &str, attrs: &[(&str, &str)], self_closing: bool) {
        self.indent();
        self.out.push('<');
        self.out.push_str(name);
        for (k, v) in attrs {
            self.out.push(' ');
            self.out.push_str(k);
            self.out.push_str("=\"");
            self.out.push_str(&escape_attr(v));
            self.out.push('"');
        }
        self.out.push_str(if self_closing { "/>\n" } else { ">\n" });
    }

    fn open(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.tag(name, attrs, false);
        self.depth += 1;
    }

    fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.tag(name, attrs, true);
    }

    fn close(&mut self, name: &str) {
        self.depth -= 1;
        self.indent();
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push_str(">\n");
    }

    fn leaf(&mut self, name: &str, text: &str) {
        self.indent();
        self.out.push('<');
        self.out.push_str(name);
        self.out.push('>');
        self.out.push_str(&escape_text(text));
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push_str(">\n");
    }

    fn point(&mut self, name: &str, p: [f64; 3]) {
        self.open(name, &[]);
        self.leaf("X", &format_coord(p[0]));
        self.leaf("Y", &format_coord(p[1]));
        self.leaf("Z", &format_coord(p[2]));
        self.close(name);
    }

    fn finish(self) -> String {
        self.out
    }
}

/// Coordinates with 9 decimals (sub-micrometre), trailing zeros trimmed, never "-0".
fn format_coord(v: f64) -> String {
    let s = format!("{v:.9}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------

fn version_xml(version: BcfVersion) -> String {
    // Same content as the official `bcf.version` of each release (the 3.0 element
    // is written self-closing: its schema allows no content, not even whitespace).
    match version {
        BcfVersion::V21 => concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Version VersionId=\"2.1\" xsi:noNamespaceSchemaLocation=\"version.xsd\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            "  <DetailedVersion>2.1</DetailedVersion>\n",
            "</Version>\n"
        )
        .to_string(),
        BcfVersion::V30 => concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Version VersionId=\"3.0\" xsi:noNamespaceSchemaLocation=\"https://raw.githubusercontent.com/buildingSMART/BCF-XML/release_3_0/Schemas/version.xsd\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"/>\n"
        )
        .to_string(),
    }
}

/// BCF 3.0 `extensions.xml`: every value used by the topics, sorted.
fn extensions_xml(issues: &[Issue], topic_type: &str, status: &str, author: &str) -> String {
    let priorities: BTreeSet<&str> = issues.iter().map(|i| i.priority.as_str()).collect();
    let labels: BTreeSet<&str> = issues
        .iter()
        .flat_map(|i| i.labels.iter().map(|l| l.as_str()))
        .collect();
    let mut x = Xml::new();
    x.open("Extensions", &[]);
    x.open("TopicTypes", &[]);
    x.leaf("TopicType", topic_type);
    x.close("TopicTypes");
    x.open("TopicStatuses", &[]);
    x.leaf("TopicStatus", status);
    x.close("TopicStatuses");
    if !priorities.is_empty() {
        x.open("Priorities", &[]);
        for p in priorities {
            x.leaf("Priority", p);
        }
        x.close("Priorities");
    }
    if !labels.is_empty() {
        x.open("TopicLabels", &[]);
        for l in labels {
            x.leaf("TopicLabel", l);
        }
        x.close("TopicLabels");
    }
    x.open("Users", &[]);
    x.leaf("User", author);
    x.close("Users");
    x.close("Extensions");
    x.finish()
}

/// Shared, per-archive values for markup files.
struct MarkupContext<'a> {
    version: BcfVersion,
    file_name: Option<&'a str>,
    project_guid: Option<&'a str>,
    topic_type: &'a str,
    status: &'a str,
    author: &'a str,
    creation_date: &'a str,
}

fn markup_xml(
    ctx: &MarkupContext,
    issue: &Issue,
    index: usize,
    element: Option<&Element>,
    viewpoint: Option<&Uuid>,
) -> String {
    let v30 = ctx.version == BcfVersion::V30;
    let guid = issue.guid.to_string();
    let mut x = Xml::new();
    x.open("Markup", &[]);

    // Header: the IFC file the topic refers to.
    if let Some(file_name) = ctx.file_name {
        let spatial = element
            .and_then(|e| e.container.as_ref())
            .map(|c| c.global_id.as_str())
            .filter(|g| is_ifc_guid(g));
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        if let Some(p) = ctx.project_guid {
            attrs.push(("IfcProject", p));
        }
        if let Some(s) = spatial {
            attrs.push(("IfcSpatialStructureElement", s));
        }
        attrs.push((if v30 { "IsExternal" } else { "isExternal" }, "true"));
        x.open("Header", &[]);
        if v30 {
            x.open("Files", &[]);
        }
        x.open("File", &attrs);
        x.leaf("Filename", file_name);
        x.close("File");
        if v30 {
            x.close("Files");
        }
        x.close("Header");
    }

    x.open(
        "Topic",
        &[
            ("Guid", &guid),
            ("TopicType", ctx.topic_type),
            ("TopicStatus", ctx.status),
        ],
    );
    x.leaf("Title", &issue.title);
    x.leaf("Priority", &issue.priority);
    if v30 {
        // `Index` is deprecated in 3.0; topic order is the archive order.
        x.open("Labels", &[]);
        for l in &issue.labels {
            x.leaf("Label", l);
        }
        x.close("Labels");
    } else {
        x.leaf("Index", &index.to_string());
        for l in &issue.labels {
            x.leaf("Labels", l);
        }
    }
    x.leaf("CreationDate", ctx.creation_date);
    x.leaf("CreationAuthor", ctx.author);
    x.leaf("Description", &issue.description);
    if v30 {
        if let Some(vp) = viewpoint {
            x.open("Viewpoints", &[]);
            x.open("ViewPoint", &[("Guid", &vp.to_string())]);
            x.leaf("Viewpoint", VIEWPOINT_FILE);
            x.close("ViewPoint");
            x.close("Viewpoints");
        }
        x.close("Topic");
    } else {
        x.close("Topic");
        if let Some(vp) = viewpoint {
            x.open("Viewpoints", &[("Guid", &vp.to_string())]);
            x.leaf("Viewpoint", VIEWPOINT_FILE);
            x.close("Viewpoints");
        }
    }
    x.close("Markup");
    x.finish()
}

fn viewpoint_xml(
    version: BcfVersion,
    guid: &Uuid,
    ifc_guid: Option<&str>,
    camera: Option<&Camera>,
) -> String {
    let mut x = Xml::new();
    x.open("VisualizationInfo", &[("Guid", &guid.to_string())]);
    if let Some(g) = ifc_guid {
        x.open("Components", &[]);
        x.open("Selection", &[]);
        x.empty("Component", &[("IfcGuid", g)]);
        x.close("Selection");
        // Everything stays visible; the related element is selected.
        x.empty("Visibility", &[("DefaultVisibility", "true")]);
        x.close("Components");
    }
    if let Some(c) = camera {
        x.open("PerspectiveCamera", &[]);
        x.point("CameraViewPoint", c.position);
        x.point("CameraDirection", c.direction);
        x.point("CameraUpVector", c.up);
        x.leaf("FieldOfView", &format_number(FIELD_OF_VIEW_DEG));
        if version == BcfVersion::V30 {
            x.leaf("AspectRatio", &format_number(ASPECT_RATIO));
        }
        x.close("PerspectiveCamera");
    }
    x.close("VisualizationInfo");
    x.finish()
}

const VIEWPOINT_FILE: &str = "viewpoint.bcfv";

/// Index of elements by GlobalId (elements sharing a GlobalId keep model order).
fn element_index(model: &Model) -> HashMap<&str, Vec<&Element>> {
    let mut map: HashMap<&str, Vec<&Element>> = HashMap::new();
    for e in &model.elements {
        map.entry(e.global_id.as_str()).or_default().push(e);
    }
    map
}

fn find_element<'m>(index: &HashMap<&str, Vec<&'m Element>>, f: &Finding) -> Option<&'m Element> {
    let candidates = index.get(f.global_id.as_deref()?)?;
    match f.step_id {
        Some(step) => candidates
            .iter()
            .find(|e| e.step_id == step)
            .or_else(|| candidates.first())
            .copied(),
        None => candidates.first().copied(),
    }
}

/// GlobalId of the IfcProject, when the reader exposed it and it is a valid IfcGuid.
fn project_guid(model: &Model) -> Option<&str> {
    let from_elements = model
        .elements
        .iter()
        .find(|e| e.ifc_type.eq_ignore_ascii_case("IfcProject"))
        .map(|e| e.global_id.as_str());
    let from_tree = || {
        model
            .info
            .spatial_tree
            .iter()
            .flat_map(|n| n.walk())
            .find(|n| n.ifc_type.eq_ignore_ascii_case("IfcProject"))
            .map(|n| n.global_id.as_str())
    };
    from_elements.or_else(from_tree).filter(|g| is_ifc_guid(g))
}

/// Build the whole archive in memory. `now` is the topics' CreationDate.
fn build_bcfzip(
    report: &RunReport,
    model: &Model,
    config: &Config,
    now: DateTime<Utc>,
) -> CoreResult<(Vec<u8>, BcfStats)> {
    let version = BcfVersion::from_config(&config.output.bcf_version);
    let strings = Strings::for_language(config.output.language);
    let issues = collect_issues(report, &strings);

    let topic_type = non_blank(strings.get("bcf.topic_type"), "Issue");
    let status = non_blank(strings.get("bcf.status_open"), "Open");
    let author = non_blank(
        config.output.bcf_author.clone(),
        &crate::config::OutputSection::default().bcf_author,
    );
    let creation_date = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let file_name = Some(report.model.file_name.as_str()).filter(|s| !s.trim().is_empty());
    let ctx = MarkupContext {
        version,
        file_name,
        project_guid: project_guid(model),
        topic_type: &topic_type,
        status: &status,
        author: &author,
        creation_date: &creation_date,
    };

    let index = element_index(model);
    let fallback_camera = match version {
        BcfVersion::V30 => model_bbox(model).and_then(|(mn, mx)| camera_for_bbox(mn, mx)),
        BcfVersion::V21 => None,
    };

    let fixed_time = zip::DateTime::from_date_and_time(2000, 1, 1, 0, 0, 0)
        .map_err(|e| CoreError::Bcf(format!("invalid zip timestamp: {e}")))?;
    let file_opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(fixed_time)
        .unix_permissions(0o644);
    let dir_opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(fixed_time)
        .unix_permissions(0o755);

    let zip_err =
        |e: zip::result::ZipError| CoreError::Bcf(format!("cannot write the BCF archive: {e}"));
    let io_err = |e: std::io::Error| CoreError::Bcf(format!("cannot write the BCF archive: {e}"));

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("bcf.version", file_opts).map_err(zip_err)?;
    zip.write_all(version_xml(version).as_bytes())
        .map_err(io_err)?;
    if version == BcfVersion::V30 {
        zip.start_file("extensions.xml", file_opts)
            .map_err(zip_err)?;
        zip.write_all(extensions_xml(&issues, &topic_type, &status, &author).as_bytes())
            .map_err(io_err)?;
    }

    let mut with_viewpoint = 0;
    for (i, issue) in issues.iter().enumerate() {
        let element = find_element(&index, issue.finding);
        let ifc_guid = issue
            .finding
            .global_id
            .as_deref()
            .filter(|g| is_ifc_guid(g));
        let element_camera = element
            .and_then(|e| e.geometry.as_ref())
            .and_then(|g| camera_for_bbox(g.bbox_min, g.bbox_max));
        let camera = element_camera.or(fallback_camera);
        let has_viewpoint = match version {
            // 2.1: camera optional, a component-only viewpoint is valid.
            BcfVersion::V21 => camera.is_some() || ifc_guid.is_some(),
            // 3.0: a viewpoint must have a camera.
            BcfVersion::V30 => camera.is_some(),
        };
        let vp_guid = has_viewpoint.then(|| viewpoint_guid(&issue.guid));

        let folder = issue.guid.to_string();
        zip.add_directory(format!("{folder}/"), dir_opts)
            .map_err(zip_err)?;
        zip.start_file(format!("{folder}/markup.bcf"), file_opts)
            .map_err(zip_err)?;
        zip.write_all(markup_xml(&ctx, issue, i + 1, element, vp_guid.as_ref()).as_bytes())
            .map_err(io_err)?;
        if let Some(vp) = &vp_guid {
            zip.start_file(format!("{folder}/{VIEWPOINT_FILE}"), file_opts)
                .map_err(zip_err)?;
            zip.write_all(viewpoint_xml(version, vp, ifc_guid, camera.as_ref()).as_bytes())
                .map_err(io_err)?;
            with_viewpoint += 1;
        }
    }
    let bytes = zip.finish().map_err(zip_err)?.into_inner();
    Ok((
        bytes,
        BcfStats {
            topics: issues.len(),
            with_viewpoint,
            version: version.as_str().to_string(),
        },
    ))
}

fn non_blank(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RuleSource, RuleStatus, SpatialRef};
    use quick_xml::events::Event;
    use std::io::Read;

    const WALL_A: &str = "2O2Fr$t4X7Zf8NOew3FLOH";
    const WALL_B: &str = "1hOSvn6df7F8_7GcBWlRGQ";

    fn finding(rule: &str, sev: Severity, gid: Option<&str>, msg: &str) -> Finding {
        Finding {
            rule_id: rule.into(),
            severity: sev,
            global_id: gid.map(Into::into),
            step_id: None,
            ifc_type: gid.map(|_| "IfcWall".into()),
            name: gid.map(|_| "Wall <A> & \"B\"".into()),
            storey: Some("Level 1".into()),
            message: msg.into(),
            expected: Some("Pset_WallCommon.FireRating".into()),
            actual: Some("missing".into()),
            fix_hint: Some("Add the property".into()),
            source: RuleSource::RulePack {
                pack: "generic".into(),
            },
        }
    }

    fn rule(id: &str, sev: Severity) -> RuleResult {
        RuleResult {
            rule_id: id.into(),
            category: "properties".into(),
            description: format!("Description of {id}"),
            severity: sev,
            status: RuleStatus::Fail,
            elements_checked: 2,
            failures: 1,
            skipped_reason: None,
            source: RuleSource::RulePack {
                pack: "generic".into(),
            },
        }
    }

    fn sample() -> (RunReport, Model) {
        let model = Model {
            info: crate::model::ModelInfo {
                file_name: "sample.ifc".into(),
                sha256: "ab".repeat(32),
                ..Default::default()
            },
            elements: vec![
                Element {
                    global_id: WALL_A.into(),
                    ifc_type: "IfcWall".into(),
                    container: Some(SpatialRef {
                        global_id: "3hOSvn6df7F8_7GcBWlRGQ".into(),
                        ifc_type: "IfcBuildingStorey".into(),
                        name: None,
                    }),
                    geometry: Some(Geometry {
                        bbox_min: [0.0, 0.0, 0.0],
                        bbox_max: [4.0, 0.2, 3.0],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Element {
                    global_id: WALL_B.into(),
                    ifc_type: "IfcWall".into(),
                    ..Default::default()
                },
            ],
            ids_results: None,
        };
        let mut report = RunReport {
            model: model.info.clone(),
            config: Config::default().to_json_value(),
            rules: vec![
                rule("PS-001", Severity::Error),
                rule("NM-002", Severity::Warning),
                rule("IN-003", Severity::Info),
            ],
            findings: vec![
                finding(
                    "PS-001",
                    Severity::Error,
                    Some(WALL_A),
                    "FireRating is missing",
                ),
                finding(
                    "NM-002",
                    Severity::Warning,
                    Some(WALL_B),
                    "Name does not match {pattern}",
                ),
                finding("IN-003", Severity::Info, Some(WALL_A), "Just information"),
                finding("PS-001", Severity::Error, None, "Model has no IfcSite"),
            ],
            ..Default::default()
        };
        report.normalise();
        (report, model)
    }

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn entries(bytes: &[u8]) -> Vec<(String, String)> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut f = zip.by_index(i).unwrap();
                let mut s = String::new();
                f.read_to_string(&mut s).unwrap();
                (f.name().to_string(), s)
            })
            .collect()
    }

    /// (local name, text, attributes) of one parsed XML element.
    type XmlNode = (String, String, Vec<(String, String)>);

    /// Parse an XML document and return (element local name, text) pairs plus attributes, failing on malformed XML.
    fn parse(xml: &str) -> Vec<XmlNode> {
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut stack: Vec<XmlNode> = Vec::new();
        let mut out = Vec::new();
        loop {
            match reader.read_event().expect("well-formed XML") {
                Event::Start(e) => {
                    let attrs = e
                        .attributes()
                        .map(|a| {
                            let a = a.unwrap();
                            (
                                String::from_utf8(a.key.as_ref().to_vec()).unwrap(),
                                a.unescape_value().unwrap().to_string(),
                            )
                        })
                        .collect();
                    stack.push((
                        String::from_utf8(e.local_name().as_ref().to_vec()).unwrap(),
                        String::new(),
                        attrs,
                    ));
                }
                Event::Empty(e) => {
                    let attrs = e
                        .attributes()
                        .map(|a| {
                            let a = a.unwrap();
                            (
                                String::from_utf8(a.key.as_ref().to_vec()).unwrap(),
                                a.unescape_value().unwrap().to_string(),
                            )
                        })
                        .collect();
                    out.push((
                        String::from_utf8(e.local_name().as_ref().to_vec()).unwrap(),
                        String::new(),
                        attrs,
                    ));
                }
                Event::Text(t) => {
                    if let Some(top) = stack.last_mut() {
                        top.1.push_str(&t.unescape().unwrap());
                    }
                }
                Event::End(_) => out.push(stack.pop().unwrap()),
                Event::Eof => break,
                _ => {}
            }
        }
        assert!(stack.is_empty());
        out
    }

    fn text_of(doc: &[XmlNode], name: &str) -> Option<String> {
        doc.iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, t, _)| t.clone())
    }

    fn attr_of(doc: &[XmlNode], name: &str, attr: &str) -> Option<String> {
        doc.iter()
            .find(|(n, _, _)| n == name)
            .and_then(|(_, _, a)| a.iter().find(|(k, _)| k == attr).map(|(_, v)| v.clone()))
    }

    #[test]
    fn bcf21_archive_structure() {
        let (report, model) = sample();
        let (bytes, stats) =
            build_bcfzip(&report, &model, &Config::default(), fixed_now()).unwrap();
        assert_eq!(stats.version, "2.1");
        assert_eq!(stats.topics, 3, "INFO findings are skipped");
        let files = entries(&bytes);
        assert_eq!(files[0].0, "bcf.version");
        let version = parse(&files[0].1);
        assert_eq!(
            attr_of(&version, "Version", "VersionId").as_deref(),
            Some("2.1")
        );
        assert_eq!(text_of(&version, "DetailedVersion").as_deref(), Some("2.1"));

        let folders: BTreeSet<&str> = files
            .iter()
            .filter(|(n, _)| n.ends_with('/'))
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(folders.len(), 3);
        let markups: Vec<&(String, String)> = files
            .iter()
            .filter(|(n, _)| n.ends_with("/markup.bcf"))
            .collect();
        assert_eq!(markups.len(), 3);

        // Report order: errors first, model-level (no GlobalId) before element findings.
        // Second topic: the error on WALL_A, which has geometry.
        let first = &markups[1];
        let folder = first.0.trim_end_matches("/markup.bcf");
        let doc = parse(&first.1);
        assert_eq!(attr_of(&doc, "Topic", "Guid").as_deref(), Some(folder));
        assert_eq!(
            attr_of(&doc, "Topic", "TopicType").as_deref(),
            Some("Issue")
        );
        assert_eq!(
            attr_of(&doc, "Topic", "TopicStatus").as_deref(),
            Some("Open")
        );
        assert_eq!(
            text_of(&doc, "Title").as_deref(),
            Some("[PS-001] FireRating is missing")
        );
        assert_eq!(text_of(&doc, "Priority").as_deref(), Some("High"));
        assert_eq!(text_of(&doc, "Index").as_deref(), Some("2"));
        assert_eq!(
            text_of(&doc, "CreationDate").as_deref(),
            Some("2026-01-02T03:04:05Z")
        );
        assert_eq!(
            text_of(&doc, "CreationAuthor").as_deref(),
            Some("verimodel@local")
        );
        assert_eq!(text_of(&doc, "Filename").as_deref(), Some("sample.ifc"));
        assert_eq!(
            attr_of(&doc, "File", "IfcSpatialStructureElement").as_deref(),
            Some("3hOSvn6df7F8_7GcBWlRGQ")
        );
        let desc = text_of(&doc, "Description").unwrap();
        assert!(desc.contains("Description of PS-001"), "{desc}");
        assert!(
            desc.contains("Wall <A> & \"B\""),
            "escaping round-trips: {desc}"
        );
        assert!(
            desc.contains(&format!("VeriModel {}", crate::VERSION)),
            "{desc}"
        );
        let labels: Vec<String> = doc
            .iter()
            .filter(|(n, _, _)| n == "Labels")
            .map(|(_, t, _)| t.clone())
            .collect();
        assert_eq!(labels, vec!["VeriModel", "properties", "PS-001"]);
        assert_eq!(
            text_of(&doc, "Viewpoint").as_deref(),
            Some("viewpoint.bcfv")
        );
        assert!(!first.1.contains("Snapshot"));

        let vp = files
            .iter()
            .find(|(n, _)| *n == format!("{folder}/viewpoint.bcfv"))
            .expect("viewpoint");
        let vdoc = parse(&vp.1);
        assert_eq!(
            attr_of(&vdoc, "Component", "IfcGuid").as_deref(),
            Some(WALL_A)
        );
        assert_eq!(
            attr_of(&vdoc, "Visibility", "DefaultVisibility").as_deref(),
            Some("true")
        );
        assert_eq!(text_of(&vdoc, "FieldOfView").as_deref(), Some("60"));
        assert_eq!(
            attr_of(&vdoc, "VisualizationInfo", "Guid"),
            attr_of(&doc, "Viewpoints", "Guid")
        );

        // First topic: model-level error (no GlobalId) -> no viewpoint at all in 2.1.
        assert_eq!(
            text_of(&parse(&markups[0].1), "Title").as_deref(),
            Some("[PS-001] Model has no IfcSite")
        );
        let model_folder = markups[0].0.trim_end_matches("/markup.bcf");
        assert!(!files
            .iter()
            .any(|(n, _)| *n == format!("{model_folder}/viewpoint.bcfv")));
        assert!(!markups[0].1.contains("<Viewpoints"));

        // Third topic: warning on WALL_B without geometry -> component-only viewpoint.
        let third = parse(&markups[2].1);
        assert_eq!(
            text_of(&third, "Title").as_deref(),
            Some("[NM-002] Name does not match {pattern}")
        );
        assert_eq!(text_of(&third, "Priority").as_deref(), Some("Normal"));
        let third_folder = markups[2].0.trim_end_matches("/markup.bcf");
        let vp3 = files
            .iter()
            .find(|(n, _)| *n == format!("{third_folder}/viewpoint.bcfv"))
            .expect("component-only viewpoint");
        assert!(!vp3.1.contains("PerspectiveCamera"));
        assert!(vp3.1.contains(WALL_B));
        assert_eq!(stats.with_viewpoint, 2);
    }

    #[test]
    fn camera_frames_the_box() {
        let c = camera_for_bbox([0.0, 0.0, 0.0], [4.0, 0.2, 3.0]).unwrap();
        let centre = [2.0, 0.1, 1.5];
        // The camera looks at the centre.
        let to_centre = [
            centre[0] - c.position[0],
            centre[1] - c.position[1],
            centre[2] - c.position[2],
        ];
        let len = (to_centre.iter().map(|v| v * v).sum::<f64>()).sqrt();
        for (t, d) in to_centre.iter().zip(c.direction.iter()) {
            assert!((t / len - d).abs() < 1e-9);
        }
        // Up is perpendicular to the direction.
        let dot: f64 = (0..3).map(|i| c.up[i] * c.direction[i]).sum();
        assert!(dot.abs() < 1e-9);
        // Far enough for the bounding sphere (radius ~2.5 m) to fit a 60 degree view.
        assert!(len > 2.5 / (30f64.to_radians().sin()));
        assert!(camera_for_bbox([0.0, 0.0, f64::NAN], [1.0, 1.0, 1.0]).is_none());
    }

    #[test]
    fn guids_are_deterministic_and_only_dates_change() {
        let (report, model) = sample();
        let cfg = Config::default();
        let (a, _) = build_bcfzip(&report, &model, &cfg, fixed_now()).unwrap();
        let (b, _) = build_bcfzip(&report, &model, &cfg, fixed_now()).unwrap();
        assert_eq!(a, b, "same input and same clock must give identical bytes");

        let later = DateTime::parse_from_rfc3339("2027-06-07T08:09:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let (c, _) = build_bcfzip(&report, &model, &cfg, later).unwrap();
        let ea = entries(&a);
        let ec = entries(&c);
        assert_eq!(ea.len(), ec.len());
        for ((na, ta), (nc, tc)) in ea.iter().zip(ec.iter()) {
            assert_eq!(na, nc, "entry names and order are stable");
            let normalised = tc.replace("2027-06-07T08:09:10Z", "2026-01-02T03:04:05Z");
            assert_eq!(ta, &normalised, "only CreationDate differs in {na}");
        }

        // The public writer also works end to end.
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("a.bcfzip");
        let stats = write_bcfzip(&report, &model, &cfg, &p1).unwrap();
        assert_eq!(stats.topics, 3);
        let names: Vec<String> = entries(&std::fs::read(&p1).unwrap())
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, ea.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn duplicate_findings_get_distinct_guids() {
        let (mut report, model) = sample();
        let dup = report.findings[0].clone();
        report.findings.push(dup);
        report.normalise();
        let strings = Strings::for_language(Language::En);
        let issues = collect_issues(&report, &strings);
        let guids: BTreeSet<Uuid> = issues.iter().map(|i| i.guid).collect();
        assert_eq!(guids.len(), issues.len());
        assert!(build_bcfzip(&report, &model, &Config::default(), fixed_now()).is_ok());
        // GUIDs do not depend on the clock or on the output language.
        let es = collect_issues(&report, &Strings::for_language(Language::Es));
        assert_eq!(
            issues.iter().map(|i| i.guid).collect::<Vec<_>>(),
            es.iter().map(|i| i.guid).collect::<Vec<_>>()
        );
        assert_eq!(issues[0].guid.get_version_num(), 5);
    }

    #[test]
    fn empty_report_gives_valid_archive_without_topics() {
        let report = RunReport::default();
        let model = Model::default();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.bcfzip");
        let stats = write_bcfzip(&report, &model, &Config::default(), &p).unwrap();
        assert_eq!(
            stats,
            BcfStats {
                topics: 0,
                with_viewpoint: 0,
                version: "2.1".into()
            }
        );
        let files = entries(&std::fs::read(&p).unwrap());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "bcf.version");
        parse(&files[0].1);
    }

    #[test]
    fn bcf30_archive_structure() {
        let (report, model) = sample();
        let mut cfg = Config::default();
        cfg.output.bcf_version = "3.0".into();
        let (bytes, stats) = build_bcfzip(&report, &model, &cfg, fixed_now()).unwrap();
        assert_eq!(stats.version, "3.0");
        assert_eq!(stats.topics, 3);
        // The model has geometry, so every topic gets a camera (model-wide fallback).
        assert_eq!(stats.with_viewpoint, 3);
        let files = entries(&bytes);
        assert_eq!(files[0].0, "bcf.version");
        assert_eq!(
            attr_of(&parse(&files[0].1), "Version", "VersionId").as_deref(),
            Some("3.0")
        );
        assert_eq!(files[1].0, "extensions.xml");
        let ext = parse(&files[1].1);
        assert_eq!(text_of(&ext, "TopicType").as_deref(), Some("Issue"));
        assert!(ext
            .iter()
            .any(|(n, t, _)| n == "TopicLabel" && t == "PS-001"));
        let markup = files
            .iter()
            .find(|(n, _)| n.ends_with("/markup.bcf"))
            .unwrap();
        let doc = parse(&markup.1);
        assert!(doc.iter().any(|(n, _, _)| n == "Files"));
        assert!(doc.iter().any(|(n, t, _)| n == "Label" && t == "PS-001"));
        assert!(attr_of(&doc, "ViewPoint", "Guid").is_some());
        assert!(text_of(&doc, "Index").is_none());
        let vp = files
            .iter()
            .find(|(n, _)| n.ends_with("/viewpoint.bcfv"))
            .unwrap();
        assert_eq!(text_of(&parse(&vp.1), "AspectRatio").as_deref(), Some("1"));

        // Without any geometry, 3.0 writes no viewpoint (a camera is mandatory there).
        let mut bare = model.clone();
        for e in &mut bare.elements {
            e.geometry = None;
        }
        let (_, stats) = build_bcfzip(&report, &bare, &cfg, fixed_now()).unwrap();
        assert_eq!(stats.with_viewpoint, 0);
    }

    #[test]
    fn unknown_version_falls_back_to_21() {
        assert_eq!(BcfVersion::from_config("1.0"), BcfVersion::V21);
        assert_eq!(BcfVersion::from_config(" 3.0 "), BcfVersion::V30);
    }

    #[test]
    fn issues_csv_matches_topics() {
        let (report, _) = sample();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("issues.csv");
        let n = write_issues_csv(&report, &p).unwrap();
        assert_eq!(n, 3);
        let first = std::fs::read(&p).unwrap();
        write_issues_csv(&report, &p).unwrap();
        assert_eq!(first, std::fs::read(&p).unwrap(), "deterministic");
        let mut rdr = csv::Reader::from_reader(first.as_slice());
        let headers: Vec<String> = rdr.headers().unwrap().iter().map(String::from).collect();
        assert_eq!(
            headers,
            [
                "severity",
                "rule_id",
                "topic_guid",
                "global_id",
                "ifc_type",
                "name",
                "storey",
                "title",
                "description",
                "expected",
                "actual",
                "fix_hint"
            ]
        );
        let rows: Vec<csv::StringRecord> = rdr.records().map(|r| r.unwrap()).collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(&rows[0][0], "ERROR");
        assert_eq!(&rows[1][7], "[PS-001] FireRating is missing");
        assert!(
            rows[0][8].contains('\n'),
            "multi-line description is quoted"
        );
        let strings = Strings::for_language(Language::En);
        let issues = collect_issues(&report, &strings);
        for (row, issue) in rows.iter().zip(issues.iter()) {
            assert_eq!(&row[2], issue.guid.to_string());
        }
    }

    #[test]
    fn template_and_escaping_helpers() {
        assert_eq!(
            fill_template(
                "[{rule}] {message}",
                &[("rule", "R"), ("message", "{rule} x")]
            ),
            "[R] {rule} x"
        );
        assert_eq!(fill_template("a {unknown} b {", &[]), "a {unknown} b {");
        assert_eq!(escape_text("a<b>&\u{1}c"), "a&lt;b&gt;&amp;c");
        assert_eq!(escape_attr("\"x'\n"), "&quot;x&apos;&#10;");
        assert!(is_ifc_guid(WALL_A));
        assert!(!is_ifc_guid("not-a-guid"));
        assert_eq!(shorten("abcdef", 4), "abc…");
        assert_eq!(format_coord(0.5773502691896258), "0.577350269");
        assert_eq!(format_coord(-0.0000000001), "0");
        assert_eq!(format_coord(2.0), "2");
    }

    /// Writes sample archives for external validation when VERIMODEL_BCF_SAMPLE_DIR is set.
    #[test]
    fn write_samples_for_external_validation() {
        let Ok(dir) = std::env::var("VERIMODEL_BCF_SAMPLE_DIR") else {
            return;
        };
        let (report, model) = sample();
        let mut cfg = Config::default();
        write_bcfzip(
            &report,
            &model,
            &cfg,
            &Path::new(&dir).join("sample21.bcfzip"),
        )
        .unwrap();
        cfg.output.bcf_version = "3.0".into();
        write_bcfzip(
            &report,
            &model,
            &cfg,
            &Path::new(&dir).join("sample30.bcfzip"),
        )
        .unwrap();
        write_bcfzip(
            &RunReport::default(),
            &Model::default(),
            &Config::default(),
            &Path::new(&dir).join("empty21.bcfzip"),
        )
        .unwrap();
    }
}
