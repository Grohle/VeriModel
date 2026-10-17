//! Report writers: report.html, report.json, failures.csv, qto.csv, qto.xlsx, qto.json.
//!
//! * [`write_report_json`] — pretty JSON, keys in struct order, no timestamps.
//! * [`write_failures_csv`] — one row per finding:
//!   `severity,rule_id,global_id,step_id,ifc_type,name,storey,message,expected,actual,fix_hint,source`.
//! * [`write_qto_csv`] / [`write_qto_xlsx`] / [`write_qto_json`] — the quantity takeoff.
//! * [`write_report_html`] — one self-contained HTML file (inline CSS, a tiny
//!   inline filter script, no external assets), labels from [`Strings`].
//! * [`write_all`] — writes everything into `out/` and lists the files.
//!
//! Every writer is deterministic: the same report gives the same bytes.

use std::fmt::Write as _;
use std::path::Path;

use crate::config::Config;
use crate::error::{CoreError, CoreResult};
use crate::i18n::Strings;
use crate::model::{
    format_number, QtoReport, RuleSource, RuleStatus, RunReport, Severity, Verdict,
};

/// Stylesheet inlined into report.html.
const REPORT_CSS: &str = include_str!("../data/report.css");

/// Maximum number of findings rendered in report.html (all of them are in failures.csv).
pub const MAX_HTML_FINDINGS: usize = 2000;

/// Fixed document creation date written into qto.xlsx so the file is reproducible.
const XLSX_FIXED_DATE: (u16, u8, u8) = (2000, 1, 1);

/// One produced file, listed on the Export screen.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OutputFile {
    /// File name inside `out/`.
    pub name: String,
    /// Plain-language description key (resolved by the UI's i18n).
    pub kind: String,
    pub bytes: u64,
}

fn csv_err(path: &Path) -> impl Fn(csv::Error) -> CoreError + '_ {
    move |e| CoreError::Report(format!("cannot write {}: {e}", path.display()))
}

fn xlsx_err(path: &Path) -> impl Fn(rust_xlsxwriter::XlsxError) -> CoreError + '_ {
    move |e| CoreError::Report(format!("cannot write {}: {e}", path.display()))
}

/// `pack:<name>` | `project_config` | `ids:<specification>/<requirement>`.
pub fn source_label(source: &RuleSource) -> String {
    match source {
        RuleSource::RulePack { pack } => format!("pack:{pack}"),
        RuleSource::ProjectConfig => "project_config".to_string(),
        RuleSource::Ids {
            specification,
            requirement,
        } => format!("ids:{specification}/{requirement}"),
    }
}

/// Per-source counts as `base_quantity=3;geometry=1`.
fn sources_cell(m: &std::collections::BTreeMap<String, usize>) -> String {
    m.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(";")
}

pub fn write_report_json(report: &RunReport, path: &Path) -> CoreResult<()> {
    let text = serde_json::to_string_pretty(report)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// The quantity takeoff alone, as pretty JSON.
pub fn write_qto_json(report: &RunReport, path: &Path) -> CoreResult<()> {
    let text = serde_json::to_string_pretty(&report.qto)?;
    std::fs::write(path, text)?;
    Ok(())
}

pub fn write_failures_csv(report: &RunReport, path: &Path) -> CoreResult<()> {
    let err = csv_err(path);
    let mut w = csv::Writer::from_path(path).map_err(&err)?;
    w.write_record([
        "severity",
        "rule_id",
        "global_id",
        "step_id",
        "ifc_type",
        "name",
        "storey",
        "message",
        "expected",
        "actual",
        "fix_hint",
        "source",
    ])
    .map_err(&err)?;
    let o = |s: &Option<String>| s.clone().unwrap_or_default();
    for f in &report.findings {
        w.write_record([
            f.severity.as_str().to_string(),
            f.rule_id.clone(),
            o(&f.global_id),
            f.step_id.map(|s| s.to_string()).unwrap_or_default(),
            o(&f.ifc_type),
            o(&f.name),
            o(&f.storey),
            f.message.clone(),
            o(&f.expected),
            o(&f.actual),
            o(&f.fix_hint),
            source_label(&f.source),
        ])
        .map_err(&err)?;
    }
    w.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// QTO table shared by CSV and XLSX
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Text(String),
    Num(f64),
    Empty,
}

/// Number format class of a column (for XLSX).
#[derive(Debug, Clone, Copy, PartialEq)]
enum ColKind {
    Text,
    Count,
    Quantity,
    Money,
}

fn qto_columns(qto: &QtoReport) -> Vec<(String, ColKind)> {
    let mut cols: Vec<(String, ColKind)> = qto
        .group_by
        .iter()
        .map(|k| (k.clone(), ColKind::Text))
        .collect();
    cols.extend(
        [
            ("count", ColKind::Count),
            ("volume_m3", ColKind::Quantity),
            ("area_m2", ColKind::Quantity),
            ("length_m", ColKind::Quantity),
            ("weight_kg", ColKind::Quantity),
            ("volume_source", ColKind::Text),
            ("area_source", ColKind::Text),
            ("length_source", ColKind::Text),
            ("rate_code", ColKind::Text),
            ("rate", ColKind::Money),
            ("unit", ColKind::Text),
            ("amount", ColKind::Money),
            ("currency", ColKind::Text),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_string(), k)),
    );
    cols
}

fn qto_rows(qto: &QtoReport) -> Vec<Vec<Cell>> {
    let mut out = Vec::with_capacity(qto.rows.len() + 1);
    for r in &qto.rows {
        let mut row: Vec<Cell> = r.group.iter().map(|g| Cell::Text(g.clone())).collect();
        // A row may carry fewer group values than keys only if the report was hand-built; pad.
        while row.len() < qto.group_by.len() {
            row.push(Cell::Empty);
        }
        row.extend([
            Cell::Num(r.count as f64),
            Cell::Num(r.volume_m3),
            Cell::Num(r.area_m2),
            Cell::Num(r.length_m),
            Cell::Num(r.weight_kg),
            Cell::Text(sources_cell(&r.volume_sources)),
            Cell::Text(sources_cell(&r.area_sources)),
            Cell::Text(sources_cell(&r.length_sources)),
        ]);
        match &r.cost {
            Some(c) => row.extend([
                Cell::Text(c.rate_code.clone()),
                Cell::Num(c.rate),
                Cell::Text(c.unit.clone()),
                Cell::Num(c.amount),
                Cell::Text(c.currency.clone()),
            ]),
            None => row.extend([
                Cell::Empty,
                Cell::Empty,
                Cell::Empty,
                Cell::Empty,
                Cell::Empty,
            ]),
        }
        out.push(row);
    }
    // TOTAL label goes in the first group column (there is always one: config validation requires a key).
    let mut total: Vec<Cell> = (0..qto.group_by.len()).map(|_| Cell::Empty).collect();
    if let Some(first) = total.first_mut() {
        *first = Cell::Text("TOTAL".into());
    }
    total.extend([
        Cell::Num(qto.total_count as f64),
        Cell::Num(qto.total_volume_m3),
        Cell::Num(qto.total_area_m2),
        Cell::Num(qto.total_length_m),
        Cell::Num(qto.total_weight_kg),
        Cell::Empty,
        Cell::Empty,
        Cell::Empty,
        Cell::Empty,
        Cell::Empty,
        Cell::Empty,
        qto.total_cost.map(Cell::Num).unwrap_or(Cell::Empty),
        qto.currency.clone().map(Cell::Text).unwrap_or(Cell::Empty),
    ]);
    out.push(total);
    out
}

pub fn write_qto_csv(report: &RunReport, path: &Path) -> CoreResult<()> {
    let err = csv_err(path);
    let mut w = csv::Writer::from_path(path).map_err(&err)?;
    let cols = qto_columns(&report.qto);
    w.write_record(cols.iter().map(|(n, _)| n.as_str()))
        .map_err(&err)?;
    for row in qto_rows(&report.qto) {
        let rec: Vec<String> = row
            .into_iter()
            .map(|c| match c {
                Cell::Text(s) => s,
                Cell::Num(n) => format_number(n),
                Cell::Empty => String::new(),
            })
            .collect();
        w.write_record(&rec).map_err(&err)?;
    }
    w.flush()?;
    Ok(())
}

pub fn write_qto_xlsx(report: &RunReport, path: &Path) -> CoreResult<()> {
    use rust_xlsxwriter::{DocProperties, ExcelDateTime, Format, FormatAlign, Workbook};
    let err = xlsx_err(path);
    let qto = &report.qto;

    let mut wb = Workbook::new();
    let created = ExcelDateTime::from_ymd(XLSX_FIXED_DATE.0, XLSX_FIXED_DATE.1, XLSX_FIXED_DATE.2)
        .map_err(&err)?;
    let props = DocProperties::new()
        .set_title("VeriModel quantity takeoff")
        .set_author(crate::TOOL_NAME)
        .set_comment(format!("{} {}", crate::TOOL_NAME, crate::VERSION))
        .set_creation_datetime(&created);
    wb.set_properties(&props);

    let bold = Format::new().set_bold();
    let bold_total = Format::new()
        .set_bold()
        .set_border_top(rust_xlsxwriter::FormatBorder::Medium);
    let fmt_for = |k: ColKind, total: bool| {
        let f = match k {
            ColKind::Count => Format::new().set_num_format("0"),
            ColKind::Quantity => Format::new().set_num_format("#,##0.000"),
            ColKind::Money => Format::new().set_num_format("#,##0.00"),
            ColKind::Text => Format::new(),
        };
        if total {
            f.set_bold()
                .set_border_top(rust_xlsxwriter::FormatBorder::Medium)
        } else {
            f
        }
    };

    // Quantities
    {
        let ws = wb.add_worksheet();
        ws.set_name("Quantities").map_err(&err)?;
        let cols = qto_columns(qto);
        for (c, (name, _)) in cols.iter().enumerate() {
            ws.write_string_with_format(0, c as u16, name, &bold)
                .map_err(&err)?;
        }
        let rows = qto_rows(qto);
        let last = rows.len().saturating_sub(1);
        for (i, row) in rows.iter().enumerate() {
            let r = (i + 1) as u32;
            let is_total = i == last;
            for (c, cell) in row.iter().enumerate() {
                let kind = cols.get(c).map(|x| x.1).unwrap_or(ColKind::Text);
                let f = fmt_for(kind, is_total);
                match cell {
                    Cell::Text(s) => {
                        ws.write_string_with_format(
                            r,
                            c as u16,
                            s,
                            if is_total { &bold_total } else { &f },
                        )
                        .map_err(&err)?;
                    }
                    Cell::Num(n) => {
                        ws.write_number_with_format(r, c as u16, *n, &f)
                            .map_err(&err)?;
                    }
                    Cell::Empty => {
                        if is_total {
                            ws.write_blank(r, c as u16, &bold_total).map_err(&err)?;
                        }
                    }
                }
            }
        }
        ws.set_freeze_panes(1, 0).map_err(&err)?;
        ws.autofit();
    }

    // Samples
    {
        let ws = wb.add_worksheet();
        ws.set_name("Samples").map_err(&err)?;
        let headers = [
            "global_id",
            "ifc_type",
            "name",
            "storey",
            "volume_m3",
            "volume_source",
            "area_m2",
            "area_source",
            "length_m",
            "length_source",
            "quantity_sets",
        ];
        for (c, h) in headers.iter().enumerate() {
            ws.write_string_with_format(0, c as u16, *h, &bold)
                .map_err(&err)?;
        }
        let qf = Format::new().set_num_format("#,##0.000");
        for (i, s) in qto.samples.iter().enumerate() {
            let r = (i + 1) as u32;
            ws.write_string(r, 0, &s.global_id).map_err(&err)?;
            ws.write_string(r, 1, &s.ifc_type).map_err(&err)?;
            ws.write_string(r, 2, s.name.as_deref().unwrap_or(""))
                .map_err(&err)?;
            ws.write_string(r, 3, s.storey.as_deref().unwrap_or(""))
                .map_err(&err)?;
            for (c, v, src) in [
                (4u16, s.volume_m3, s.volume_source),
                (6, s.area_m2, s.area_source),
                (8, s.length_m, s.length_source),
            ] {
                if let Some(v) = v {
                    ws.write_number_with_format(r, c, v, &qf).map_err(&err)?;
                }
                ws.write_string(r, c + 1, src.label()).map_err(&err)?;
            }
            ws.write_string(r, 10, s.quantity_sets.join("; "))
                .map_err(&err)?;
        }
        ws.set_freeze_panes(1, 0).map_err(&err)?;
        ws.autofit();
    }

    // Method
    {
        let ws = wb.add_worksheet();
        ws.set_name("Method").map_err(&err)?;
        let wrap = Format::new().set_text_wrap().set_align(FormatAlign::Top);
        ws.set_column_width(0, 30).map_err(&err)?;
        ws.set_column_width(1, 110).map_err(&err)?;
        let facts: Vec<(&str, String)> = vec![
            ("Method", qto.method.clone()),
            ("Group by", qto.group_by.join(", ")),
            ("Elements measured", qto.total_count.to_string()),
            (
                "Elements without any quantity",
                qto.elements_without_quantities.to_string(),
            ),
            (
                "Units",
                "volume m3, area m2, length m, weight kg (SI)".to_string(),
            ),
            (
                "Total cost",
                match (qto.total_cost, &qto.currency) {
                    (Some(t), Some(c)) => format!("{t:.2} {c}"),
                    (Some(t), None) => format!("{t:.2}"),
                    _ => String::new(),
                },
            ),
            (
                "Generated by",
                format!("{} {}", crate::TOOL_NAME, crate::VERSION),
            ),
            ("Source model", report.model.file_name.clone()),
            ("Source SHA-256", report.model.sha256.clone()),
        ];
        for (i, (k, v)) in facts.iter().enumerate() {
            ws.write_string_with_format(i as u32, 0, *k, &bold.clone().set_align(FormatAlign::Top))
                .map_err(&err)?;
            ws.write_string_with_format(i as u32, 1, v, &wrap)
                .map_err(&err)?;
        }
    }

    wb.save(path).map_err(&err)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// HTML
// ---------------------------------------------------------------------------

/// Escape text for an HTML text node.
fn esc(s: &str) -> String {
    html_escape::encode_text(s).to_string()
}

/// Escape text for a double-quoted HTML attribute.
fn attr(s: &str) -> String {
    html_escape::encode_double_quoted_attribute(s).to_string()
}

const DASH: &str = "<span class=\"empty\">—</span>";

fn opt_cell(s: &Option<String>) -> String {
    match s.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => esc(v),
        _ => DASH.to_string(),
    }
}

/// Quantity for display: three decimals at most.
fn qty(x: f64) -> String {
    format_number((x * 1000.0).round() / 1000.0)
}

fn money(x: f64) -> String {
    format!("{x:.2}")
}

fn severity_badge(sev: Severity, strings: &Strings) -> String {
    let class = match sev {
        Severity::Error => "b-error",
        Severity::Warning => "b-warning",
        Severity::Info => "b-info",
    };
    format!(
        "<span class=\"badge {class}\">{}</span>",
        esc(&strings.get(&format!("severity.{}", sev.as_str())))
    )
}

fn status_badge(status: RuleStatus, strings: &Strings) -> String {
    let (key, class) = match status {
        RuleStatus::Pass => ("PASS", "b-pass"),
        RuleStatus::Fail => ("FAIL", "b-fail"),
        RuleStatus::Skipped => ("SKIPPED", "b-skipped"),
        RuleStatus::NotApplicable => ("NOT_APPLICABLE", "b-na"),
    };
    format!(
        "<span class=\"badge {class}\">{}</span>",
        esc(&strings.get(&format!("status.{key}")))
    )
}

fn pass_fail_badge(passed: bool, strings: &Strings) -> String {
    status_badge(
        if passed {
            RuleStatus::Pass
        } else {
            RuleStatus::Fail
        },
        strings,
    )
}

fn verdict_badge(verdict: Option<Verdict>, strings: &Strings) -> String {
    let (key, class) = match verdict {
        Some(Verdict::Pass) => ("PASS", "b-pass"),
        Some(Verdict::PassWithWarnings) => ("PASS_WITH_WARNINGS", "b-pww"),
        Some(Verdict::Fail) => ("FAIL", "b-fail"),
        None => {
            return format!(
                "<span class=\"badge big b-none\">{}</span>",
                esc(&strings.get("report.no_rules"))
            )
        }
    };
    format!(
        "<span class=\"badge big {class}\">{}</span>",
        esc(&strings.get(&format!("verdict.{key}")))
    )
}

fn group_label(key: &str, strings: &Strings) -> String {
    let k = match key {
        "type" => "report.ifc_type",
        "storey" => "report.storey",
        "classification" => "report.classification",
        "material" => "report.material",
        "predefined_type" => "report.predefined_type",
        "type_name" => "report.type_name",
        other => return other.to_string(),
    };
    strings.get(k)
}

fn source_name(src: crate::model::QuantitySource, strings: &Strings) -> String {
    strings.get(&format!("report.source.{}", src.label()))
}

fn sources_tooltip(m: &std::collections::BTreeMap<String, usize>, strings: &Strings) -> String {
    m.iter()
        .map(|(k, v)| format!("{}: {v}", strings.get(&format!("report.source.{k}"))))
        .collect::<Vec<_>>()
        .join("; ")
}

fn units_text(report: &RunReport) -> String {
    let u = &report.model.units;
    let parts: Vec<&str> = [&u.length_unit, &u.area_unit, &u.volume_unit]
        .iter()
        .filter_map(|x| x.as_deref())
        .collect();
    let system = match u.system {
        crate::model::UnitSystem::Metric => "metric",
        crate::model::UnitSystem::Imperial => "imperial",
    };
    if parts.is_empty() {
        format!("({system})")
    } else {
        format!("{} ({system})", parts.join(" · "))
    }
}

/// Render the full HTML report as a string.
pub fn render_report_html(report: &RunReport, strings: &Strings) -> String {
    let t = |k: &str| esc(&strings.get(k));
    let lang = report
        .config
        .pointer("/output/language")
        .and_then(|v| v.as_str())
        .filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !s.is_empty())
        .unwrap_or("en");
    let m = &report.model;
    let s = &report.summary;
    let mut h =
        String::with_capacity(64 * 1024 + report.findings.len().min(MAX_HTML_FINDINGS) * 600);

    let _ = write!(
        h,
        "<!DOCTYPE html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"generator\" content=\"{} {}\">\n<title>{} — {}</title>\n<style>\n{}</style>\n</head>\n<body>\n<main>\n",
        crate::TOOL_NAME,
        attr(crate::VERSION),
        t("report.title"),
        esc(&m.file_name),
        REPORT_CSS
    );

    // Header
    let _ = write!(
        h,
        "<header class=\"top\">\n<h1>{}</h1>\n<p class=\"tagline\">{}</p>\n",
        t("report.title"),
        t("report.subtitle")
    );
    h.push_str("<section aria-labelledby=\"h-model\">\n");
    let _ = writeln!(
        h,
        "<h2 id=\"h-model\">{}</h2>\n<dl class=\"facts\">",
        t("report.model")
    );
    let mut fact = |label: &str, value: String| {
        let _ = writeln!(h, "<dt>{}</dt><dd>{value}</dd>", t(label));
    };
    fact("report.file", esc(&m.file_name));
    if let Some(p) = m.project_name.as_deref().filter(|p| !p.trim().is_empty()) {
        fact("report.project", esc(p));
    }
    let schema = match &m.mvd {
        Some(mvd) if !mvd.trim().is_empty() => format!("{} · {}", esc(&m.schema), esc(mvd)),
        _ => esc(&m.schema),
    };
    fact("report.schema", schema);
    fact(
        "report.sha256",
        format!("<span class=\"mono\">{}</span>", esc(&m.sha256)),
    );
    fact("report.units", esc(&units_text(report)));
    fact("report.element_count", m.element_count.to_string());
    fact(
        "report.storeys",
        if m.storeys.is_empty() {
            DASH.into()
        } else {
            esc(&m.storeys.join(", "))
        },
    );
    fact(
        "report.disciplines",
        if m.disciplines.is_empty() {
            DASH.into()
        } else {
            esc(&m.disciplines.join(", "))
        },
    );
    fact("report.authoring_tool", opt_cell(&m.authoring_tool));
    h.push_str("</dl>\n");
    if !m.reader_warnings.is_empty() {
        let _ = writeln!(h, "<h3>{}</h3>\n<ul>", t("report.reader_warnings"));
        for w in &m.reader_warnings {
            let _ = writeln!(h, "<li>{}</li>", esc(w));
        }
        h.push_str("</ul>\n");
    }
    h.push_str("</section>\n</header>\n");

    // Summary
    let _ = writeln!(h, "<section aria-labelledby=\"h-summary\">\n<h2 id=\"h-summary\">{}</h2>\n<div class=\"cards\">", t("report.summary"));
    let _ = writeln!(h, "<div class=\"card score\"><div class=\"label\">{}</div><div class=\"value\">{}</div></div>", t("report.score"), s.score);
    let _ = writeln!(
        h,
        "<div class=\"card verdict\"><div class=\"label\">{}</div><div class=\"value\">{}</div></div>",
        t("report.verdict"),
        verdict_badge(s.verdict, strings)
    );
    for (class, label, n) in [
        ("checked", "report.elements_checked", s.elements_checked),
        ("errors", "report.errors", s.errors),
        ("warnings", "report.warnings", s.warnings),
        ("infos", "report.infos", s.infos),
    ] {
        let _ = writeln!(h, "<div class=\"card {class}\"><div class=\"label\">{}</div><div class=\"value\">{n}</div></div>", t(label));
    }
    h.push_str("</div>\n");
    let rule_text = if s.verdict_rule.trim().is_empty() {
        crate::score::VERDICT_RULE
    } else {
        s.verdict_rule.as_str()
    };
    let _ = writeln!(
        h,
        "<div class=\"verdict-rule\"><strong>{}</strong>{}</div>\n</section>",
        t("report.verdict_rule"),
        esc(rule_text)
    );

    // Rules
    let _ = writeln!(
        h,
        "<section aria-labelledby=\"h-rules\">\n<h2 id=\"h-rules\">{}</h2>",
        t("report.rules")
    );
    let _ = writeln!(
        h,
        "<p>{} <strong>{}</strong> · {} <strong>{}</strong> · {} <strong>{}</strong> · {} <strong>{}</strong></p>",
        t("report.rules_passed"),
        s.rules_passed,
        t("report.rules_failed"),
        s.rules_failed,
        t("report.rules_skipped"),
        s.rules_skipped,
        t("report.rules_not_applicable"),
        s.rules_not_applicable
    );
    if report.rules.is_empty() {
        let _ = writeln!(h, "<p class=\"empty\">{}</p>", t("report.no_rules"));
    } else {
        let _ = writeln!(
            h,
            "<div class=\"table-wrap\"><table>\n<thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th class=\"num\">{}</th><th class=\"num\">{}</th><th>{}</th></tr></thead>\n<tbody>",
            t("report.rule_id"),
            t("report.category"),
            t("report.description"),
            t("report.severity"),
            t("report.status"),
            t("report.checked"),
            t("report.failures_count"),
            t("report.reason")
        );
        for r in &report.rules {
            let _ = writeln!(
                h,
                "<tr><td class=\"gid\" title=\"{}\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td></tr>",
                attr(&source_label(&r.source)),
                esc(&r.rule_id),
                esc(&r.category),
                esc(&r.description),
                severity_badge(r.severity, strings),
                status_badge(r.status, strings),
                r.elements_checked,
                r.failures,
                opt_cell(&r.skipped_reason)
            );
        }
        h.push_str("</tbody></table></div>\n");
    }
    let _ = writeln!(
        h,
        "<p class=\"note\">{}</p>\n</section>",
        t("report.skipped_note")
    );

    // Failures
    let _ = writeln!(
        h,
        "<section aria-labelledby=\"h-failures\">\n<h2 id=\"h-failures\">{} ({})</h2>",
        t("report.failures"),
        report.findings.len()
    );
    if report.findings.is_empty() {
        let _ = writeln!(h, "<p class=\"empty\">{}</p>", t("report.no_failures"));
    } else {
        let shown = report.findings.len().min(MAX_HTML_FINDINGS);
        if report.findings.len() > shown {
            let total = report.findings.len().to_string();
            let more = (report.findings.len() - shown).to_string();
            let _ = writeln!(
                h,
                "<p class=\"note\"><strong>{}</strong></p>",
                esc(&strings.fmt(
                    "report.failures_truncated",
                    &[
                        ("shown", &shown.to_string()),
                        ("total", &total),
                        ("more", &more)
                    ]
                ))
            );
        }
        let count_tpl = strings.get("report.filter_count");
        let count_now = strings.fmt(
            "report.filter_count",
            &[("shown", &shown.to_string()), ("total", &shown.to_string())],
        );
        let _ = writeln!(
            h,
            "<div class=\"filters\"><input id=\"f-q\" type=\"search\" placeholder=\"{}\" aria-label=\"{}\">\
             <select id=\"f-sev\" aria-label=\"{}\"><option value=\"\">{}</option><option value=\"ERROR\">{}</option><option value=\"WARNING\">{}</option><option value=\"INFO\">{}</option></select>\
             <span class=\"count\" id=\"f-count\" data-tpl=\"{}\">{}</span></div>",
            attr(&strings.get("report.filter_placeholder")),
            attr(&strings.get("report.filter_placeholder")),
            attr(&strings.get("report.severity")),
            t("report.filter_all_severities"),
            t("severity.ERROR"),
            t("severity.WARNING"),
            t("severity.INFO"),
            attr(&count_tpl),
            esc(&count_now)
        );
        let _ = writeln!(
            h,
            "<div class=\"table-wrap\"><table id=\"failures-table\">\n<thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead>\n<tbody>",
            t("report.severity"),
            t("report.rule_id"),
            t("report.global_id"),
            t("report.ifc_type"),
            t("report.name"),
            t("report.storey"),
            t("report.message"),
            t("report.expected"),
            t("report.actual"),
            t("report.fix_hint")
        );
        for f in report.findings.iter().take(shown) {
            let _ = writeln!(
                h,
                "<tr data-sev=\"{}\"><td>{}</td><td class=\"gid\" title=\"{}\">{}</td><td class=\"gid\"{}>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"hint\">{}</td></tr>",
                f.severity.as_str(),
                severity_badge(f.severity, strings),
                attr(&source_label(&f.source)),
                esc(&f.rule_id),
                f.step_id.map(|id| format!(" title=\"#{id}\"")).unwrap_or_default(),
                opt_cell(&f.global_id),
                opt_cell(&f.ifc_type),
                opt_cell(&f.name),
                opt_cell(&f.storey),
                esc(&f.message),
                opt_cell(&f.expected),
                opt_cell(&f.actual),
                opt_cell(&f.fix_hint)
            );
        }
        h.push_str("</tbody></table></div>\n");
        h.push_str(FILTER_SCRIPT);
    }
    h.push_str("</section>\n");

    render_qto(&mut h, &report.qto, strings);
    if let Some(ids) = &report.ids {
        render_ids(&mut h, ids, strings);
    }

    // Clash
    let c = &report.clash;
    let _ = writeln!(
        h,
        "<section aria-labelledby=\"h-clash\">\n<h2 id=\"h-clash\">{}</h2>\n<p>{}</p>",
        t("report.clash"),
        esc(&c.message)
    );
    if !c.clashes.is_empty() {
        h.push_str("<div class=\"table-wrap\"><table>\n<thead><tr><th>GlobalId A</th><th>GlobalId B</th><th>Type</th><th class=\"num\">m</th></tr></thead>\n<tbody>\n");
        for x in &c.clashes {
            let _ = writeln!(
                h,
                "<tr><td class=\"gid\">{}</td><td class=\"gid\">{}</td><td>{}</td><td class=\"num\">{}</td></tr>",
                esc(&x.global_id_a),
                esc(&x.global_id_b),
                esc(&x.clash_type),
                qty(x.distance_m)
            );
        }
        h.push_str("</tbody></table></div>\n");
    }
    h.push_str("</section>\n");

    // Footer
    let _ = write!(
        h,
        "<footer>\n<p>{} {} {}</p>\n<p>{}</p>\n</footer>\n</main>\n</body>\n</html>\n",
        t("report.generated_by"),
        esc(&report.tool.name),
        esc(&report.tool.version),
        t("report.bcf_link")
    );
    h
}

/// Client-side filter for the failures table (inline, no dependencies).
const FILTER_SCRIPT: &str = "<script>\n(function(){var q=document.getElementById('f-q'),s=document.getElementById('f-sev'),c=document.getElementById('f-count');if(!q||!s)return;var rows=Array.prototype.slice.call(document.querySelectorAll('#failures-table tbody tr'));var tpl=c?c.getAttribute('data-tpl'):'';function run(){var t=q.value.toLowerCase(),v=s.value,n=0;rows.forEach(function(r){var ok=(!v||r.getAttribute('data-sev')===v)&&(!t||r.textContent.toLowerCase().indexOf(t)>=0);r.style.display=ok?'':'none';if(ok)n++;});if(c)c.textContent=tpl.replace('{shown}',n).replace('{total}',rows.length);}q.addEventListener('input',run);s.addEventListener('change',run);})();\n</script>\n";

fn render_qto(h: &mut String, q: &QtoReport, strings: &Strings) {
    let t = |k: &str| esc(&strings.get(k));
    let _ = writeln!(
        h,
        "<section aria-labelledby=\"h-qto\">\n<h2 id=\"h-qto\">{}</h2>",
        t("report.qto")
    );
    if q.rows.is_empty() {
        let _ = writeln!(h, "<p class=\"empty\">{}</p>", t("report.no_quantities"));
    } else {
        let has_cost = q.rows.iter().any(|r| r.cost.is_some());
        h.push_str("<div class=\"table-wrap\"><table>\n<thead><tr>");
        for k in &q.group_by {
            let _ = write!(h, "<th>{}</th>", esc(&group_label(k, strings)));
        }
        let _ = write!(
            h,
            "<th class=\"num\">{}</th><th class=\"num\">{}</th><th class=\"num\">{}</th><th class=\"num\">{}</th><th class=\"num\">{}</th>",
            t("report.count"),
            t("report.volume"),
            t("report.area"),
            t("report.length"),
            t("report.weight")
        );
        if has_cost {
            let _ = write!(
                h,
                "<th>{}</th><th class=\"num\">{}</th><th class=\"num\">{}</th>",
                t("report.rate_code"),
                t("report.rate"),
                t("report.amount")
            );
        }
        h.push_str("</tr></thead>\n<tbody>\n");
        for r in &q.rows {
            h.push_str("<tr>");
            for i in 0..q.group_by.len() {
                let _ = write!(
                    h,
                    "<td>{}</td>",
                    r.group
                        .get(i)
                        .map(|g| esc(g))
                        .unwrap_or_else(|| DASH.into())
                );
            }
            let _ = write!(
                h,
                "<td class=\"num\">{}</td><td class=\"num\" title=\"{}\">{}</td><td class=\"num\" title=\"{}\">{}</td><td class=\"num\" title=\"{}\">{}</td><td class=\"num\">{}</td>",
                r.count,
                attr(&sources_tooltip(&r.volume_sources, strings)),
                qty(r.volume_m3),
                attr(&sources_tooltip(&r.area_sources, strings)),
                qty(r.area_m2),
                attr(&sources_tooltip(&r.length_sources, strings)),
                qty(r.length_m),
                qty(r.weight_kg)
            );
            if has_cost {
                match &r.cost {
                    Some(c) => {
                        let _ = write!(
                            h,
                            "<td title=\"{}\">{}</td><td class=\"num\">{} {}/{}</td><td class=\"num\">{} {}</td>",
                            attr(&c.rate_description),
                            esc(&c.rate_code),
                            money(c.rate),
                            esc(&c.currency),
                            esc(&c.unit),
                            money(c.amount),
                            esc(&c.currency)
                        );
                    }
                    None => {
                        let _ = write!(h, "<td>{DASH}</td><td class=\"num\">{DASH}</td><td class=\"num\">{DASH}</td>");
                    }
                }
            }
            h.push_str("</tr>\n");
        }
        // Totals
        h.push_str("<tr class=\"total\">");
        if !q.group_by.is_empty() {
            let _ = write!(
                h,
                "<td colspan=\"{}\">{}</td>",
                q.group_by.len(),
                t("report.total")
            );
        }
        let _ = write!(
            h,
            "<td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>",
            q.total_count,
            qty(q.total_volume_m3),
            qty(q.total_area_m2),
            qty(q.total_length_m),
            qty(q.total_weight_kg)
        );
        if has_cost {
            let total = match (q.total_cost, &q.currency) {
                (Some(v), Some(c)) => format!("{} {}", money(v), esc(c)),
                (Some(v), None) => money(v),
                _ => DASH.to_string(),
            };
            let _ = write!(h, "<td></td><td></td><td class=\"num\">{total}</td>");
        }
        h.push_str("</tr>\n</tbody></table></div>\n");
        if has_cost {
            let mut sources: Vec<&str> = q
                .rows
                .iter()
                .filter_map(|r| r.cost.as_ref())
                .map(|c| c.source.as_str())
                .collect();
            sources.sort_unstable();
            sources.dedup();
            let _ = writeln!(
                h,
                "<p class=\"note\">{}: {}</p>",
                t("report.rate_source"),
                esc(&sources.join(", "))
            );
        }
    }
    let _ = writeln!(
        h,
        "<p class=\"note\">{}: <strong>{}</strong></p>",
        t("report.elements_without_quantities"),
        q.elements_without_quantities
    );
    let _ = writeln!(
        h,
        "<h3>{}</h3>\n<p class=\"method\">{}</p>",
        t("report.method"),
        esc(&q.method)
    );

    if !q.samples.is_empty() {
        let _ = writeln!(
            h,
            "<h3>{}</h3>\n<div class=\"table-wrap\"><table>\n<thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th class=\"num\">{}</th><th class=\"num\">{}</th><th class=\"num\">{}</th><th>{}</th></tr></thead>\n<tbody>",
            t("report.samples"),
            t("report.global_id"),
            t("report.ifc_type"),
            t("report.name"),
            t("report.storey"),
            t("report.volume"),
            t("report.area"),
            t("report.length"),
            t("report.quantity_sets")
        );
        let cell = |v: Option<f64>, src: crate::model::QuantitySource| {
            let value = v.map(qty).unwrap_or_else(|| DASH.to_string());
            format!(
                "<td class=\"num\">{value}<br><span class=\"src\">{}</span></td>",
                esc(&source_name(src, strings))
            )
        };
        for s in &q.samples {
            let _ = writeln!(
                h,
                "<tr><td class=\"gid\">{}</td><td>{}</td><td>{}</td><td>{}</td>{}{}{}<td>{}</td></tr>",
                esc(&s.global_id),
                esc(&s.ifc_type),
                opt_cell(&s.name),
                opt_cell(&s.storey),
                cell(s.volume_m3, s.volume_source),
                cell(s.area_m2, s.area_source),
                cell(s.length_m, s.length_source),
                if s.quantity_sets.is_empty() { DASH.to_string() } else { esc(&s.quantity_sets.join(", ")) }
            );
        }
        h.push_str("</tbody></table></div>\n");
    }
    h.push_str("</section>\n");
}

fn render_ids(h: &mut String, ids: &crate::model::IdsReport, strings: &Strings) {
    let t = |k: &str| esc(&strings.get(k));
    let _ = writeln!(
        h,
        "<section aria-labelledby=\"h-ids\">\n<h2 id=\"h-ids\">{}</h2>",
        t("report.ids")
    );
    let title = match &ids.title {
        Some(ti) if !ti.trim().is_empty() => format!("{} — {}", esc(&ids.ids_file), esc(ti)),
        _ => esc(&ids.ids_file),
    };
    let _ = writeln!(
        h,
        "<dl class=\"facts\"><dt>{}</dt><dd>{title}</dd></dl>",
        t("report.file")
    );
    let _ = writeln!(
        h,
        "<p>{}</p>",
        esc(&strings.fmt(
            "report.ids_summary",
            &[
                ("passed", &ids.specifications_passed.to_string()),
                ("total", &ids.specifications_total.to_string())
            ]
        ))
    );
    let _ = writeln!(
        h,
        "<div class=\"table-wrap\"><table>\n<thead><tr><th>{}</th><th>{}</th><th class=\"num\">{}</th><th>{}</th><th class=\"num\">{}</th></tr></thead>\n<tbody>",
        t("report.ids_specification"),
        t("report.ids_requirement"),
        t("report.ids_applicable"),
        t("report.ids_result"),
        t("report.ids_failed_entities")
    );
    for spec in &ids.specifications {
        let mut spec_cell = format!("<strong>{}</strong>", esc(&spec.name));
        if let Some(d) = spec.description.as_deref().filter(|d| !d.trim().is_empty()) {
            let _ = write!(spec_cell, "<br><span class=\"src\">{}</span>", esc(d));
        }
        if spec.requirements.is_empty() {
            let _ = writeln!(
                h,
                "<tr><td>{spec_cell}</td><td>{DASH}</td><td class=\"num\">{}</td><td>{}</td><td class=\"num\">0</td></tr>",
                spec.applicable_count,
                pass_fail_badge(spec.passed, strings)
            );
        }
        for req in &spec.requirements {
            let _ = writeln!(
                h,
                "<tr><td>{spec_cell}</td><td><blockquote>“{}”</blockquote></td><td class=\"num\">{}</td><td>{}</td><td class=\"num\">{}</td></tr>",
                esc(&req.description),
                spec.applicable_count,
                pass_fail_badge(req.passed, strings),
                req.failed_entities.len()
            );
        }
    }
    h.push_str("</tbody></table></div>\n</section>\n");
}

pub fn write_report_html(report: &RunReport, strings: &Strings, path: &Path) -> CoreResult<()> {
    std::fs::write(path, render_report_html(report, strings))?;
    Ok(())
}

/// Write every report file into `out_dir` and return the list.
pub fn write_all(
    report: &RunReport,
    config: &Config,
    out_dir: &Path,
) -> CoreResult<Vec<OutputFile>> {
    let strings = Strings::for_language(config.output.language);
    let mut files = Vec::new();
    let mut add = |name: &str, kind: &str| -> CoreResult<()> {
        let p = out_dir.join(name);
        let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        files.push(OutputFile {
            name: name.into(),
            kind: kind.into(),
            bytes,
        });
        Ok(())
    };
    write_report_json(report, &out_dir.join("report.json"))?;
    add("report.json", "report_json")?;
    write_report_html(report, &strings, &out_dir.join("report.html"))?;
    add("report.html", "report_html")?;
    write_failures_csv(report, &out_dir.join("failures.csv"))?;
    add("failures.csv", "failures_csv")?;
    write_qto_csv(report, &out_dir.join("qto.csv"))?;
    add("qto.csv", "qto_csv")?;
    write_qto_xlsx(report, &out_dir.join("qto.xlsx"))?;
    add("qto.xlsx", "qto_xlsx")?;
    write_qto_json(report, &out_dir.join("qto.json"))?;
    add("qto.json", "qto_json")?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CostLine, QtoRow};

    #[test]
    fn source_labels() {
        assert_eq!(
            source_label(&RuleSource::RulePack {
                pack: "base".into()
            }),
            "pack:base"
        );
        assert_eq!(source_label(&RuleSource::ProjectConfig), "project_config");
        assert_eq!(
            source_label(&RuleSource::Ids {
                specification: "Walls".into(),
                requirement: "FireRating".into()
            }),
            "ids:Walls/FireRating"
        );
    }

    #[test]
    fn qto_rows_end_with_total_and_match_columns() {
        let mut q = QtoReport {
            group_by: vec!["type".into(), "storey".into()],
            ..Default::default()
        };
        q.rows.push(QtoRow {
            group: vec!["IfcWall".into(), "L1".into()],
            count: 2,
            volume_m3: 1.5,
            cost: Some(CostLine {
                rate_code: "R".into(),
                rate: 2.0,
                amount: 3.0,
                unit: "m3".into(),
                currency: "EUR".into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        q.total_count = 2;
        q.total_cost = Some(3.0);
        let cols = qto_columns(&q);
        let rows = qto_rows(&q);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.len() == cols.len()));
        assert_eq!(rows[1][0], Cell::Text("TOTAL".into()));
    }

    #[test]
    fn html_escapes_and_truncates() {
        let mut r = RunReport::default();
        r.model.file_name = "a<b>.ifc".into();
        let html = render_report_html(&r, &Strings::for_language(crate::config::Language::En));
        assert!(html.contains("a&lt;b&gt;.ifc"));
        assert!(!html.contains("a<b>.ifc"));
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "report must be self-contained"
        );
    }
}
