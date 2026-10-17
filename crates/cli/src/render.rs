//! Plain-text rendering of results. Pure functions so they can be unit-tested.

use verimodel_core::model::{ModelInfo, QtoReport, RuleStatus, Summary, Verdict};
use verimodel_core::run::RunOutcome;

/// Exit code for a verdict: 0 for PASS / PASS WITH WARNINGS, 1 for FAIL.
pub fn exit_code_for(verdict: Option<Verdict>) -> i32 {
    match verdict {
        Some(Verdict::Pass) | Some(Verdict::PassWithWarnings) => 0,
        Some(Verdict::Fail) | None => crate::error::EXIT_FAIL,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Human readable run summary.
pub fn render_summary(outcome: &RunOutcome) -> String {
    let r = &outcome.report;
    let s: &Summary = &r.summary;
    let verdict = s.verdict.map(|v| v.as_str()).unwrap_or("UNKNOWN");
    let mut out = String::new();
    out.push_str(&format!("{} {} - {}\n", verimodel_core::TOOL_NAME, verimodel_core::VERSION, r.model.file_name));
    out.push_str(&format!("Schema {}, {}\n\n", r.model.schema, plural(r.model.element_count, "element", "elements")));
    out.push_str(&format!("Verdict: {verdict}   Score: {}/100\n", s.score));
    if !s.verdict_rule.is_empty() {
        out.push_str(&format!("  ({})\n", s.verdict_rule));
    }
    out.push_str(&format!(
        "Findings: {}, {}, {}\n",
        plural(s.errors, "error", "errors"),
        plural(s.warnings, "warning", "warnings"),
        plural(s.infos, "info", "infos"),
    ));
    out.push_str(&format!(
        "Rules: {} passed, {} failed, {} skipped ({} total)\n",
        s.rules_passed, s.rules_failed, s.rules_skipped, s.rules_total
    ));

    let failed: Vec<_> = r.rules.iter().filter(|x| x.status == RuleStatus::Fail).collect();
    if !failed.is_empty() {
        out.push_str("\nFailed rules:\n");
        for x in failed.iter().take(15) {
            out.push_str(&format!(
                "  [{}] {} - {} ({} on {} checked)\n",
                x.severity.as_str(),
                x.rule_id,
                x.description,
                plural(x.failures, "failure", "failures"),
                x.elements_checked
            ));
        }
        if failed.len() > 15 {
            out.push_str(&format!("  ... and {} more (see report.html)\n", failed.len() - 15));
        }
    }
    let skipped: Vec<_> = r.rules.iter().filter(|x| x.status == RuleStatus::Skipped).collect();
    if !skipped.is_empty() {
        out.push_str("\nSkipped rules (not counted as passed):\n");
        for x in skipped.iter().take(10) {
            out.push_str(&format!("  {} - {}\n", x.rule_id, x.skipped_reason.clone().unwrap_or_else(|| "no reason given".into())));
        }
    }

    out.push_str(&format!("\nOutput folder: {}\n", outcome.out_dir.display()));
    for f in &outcome.files {
        out.push_str(&format!("  {:<16} {:>10}\n", f.name, human_bytes(f.bytes)));
    }
    out
}

/// Summary lines for `inspect`.
pub fn render_info(info: &ModelInfo) -> String {
    let mut out = String::new();
    out.push_str(&format!("File:        {}\n", info.file_name));
    out.push_str(&format!("Size:        {}\n", human_bytes(info.file_size_bytes)));
    out.push_str(&format!("SHA-256:     {}\n", info.sha256));
    out.push_str(&format!("Schema:      {}\n", info.schema));
    if let Some(p) = &info.project_name {
        out.push_str(&format!("Project:     {p}\n"));
    }
    if let Some(t) = &info.authoring_tool {
        out.push_str(&format!("Authored in: {t}\n"));
    }
    let u = &info.units;
    out.push_str(&format!(
        "Units:       {} ({})\n",
        u.length_unit.clone().unwrap_or_else(|| "not declared".into()),
        match u.system {
            verimodel_core::model::UnitSystem::Metric => "metric",
            verimodel_core::model::UnitSystem::Imperial => "imperial",
        }
    ));
    out.push_str(&format!("Elements:    {}\n", info.element_count));
    out.push_str(&format!("Storeys:     {}\n", if info.storeys.is_empty() { "none found".to_string() } else { info.storeys.join(", ") }));
    out.push_str(&format!("Disciplines: {}\n", if info.disciplines.is_empty() { "unknown".to_string() } else { info.disciplines.join(", ") }));
    if !info.type_counts.is_empty() {
        out.push_str("\nElement types:\n");
        let mut rows: Vec<(&String, &usize)> = info.type_counts.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (t, n) in rows {
            out.push_str(&format!("  {:<36} {:>7}\n", t, n));
        }
    }
    if !info.reader_warnings.is_empty() {
        out.push_str("\nReader notes:\n");
        for w in &info.reader_warnings {
            out.push_str(&format!("  - {w}\n"));
        }
    }
    out
}

/// Quantity takeoff as an aligned text table.
pub fn render_qto(q: &QtoReport) -> String {
    let mut headers: Vec<String> = q.group_by.clone();
    if headers.is_empty() {
        headers.push("group".into());
    }
    let n_group = headers.len();
    let has_cost = q.rows.iter().any(|r| r.cost.is_some());
    for h in ["count", "volume m3", "area m2", "length m"] {
        headers.push(h.into());
    }
    if has_cost {
        headers.push(format!("cost {}", q.currency.clone().unwrap_or_default()).trim().to_string());
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    for r in &q.rows {
        let mut cells: Vec<String> = (0..n_group).map(|i| r.group.get(i).cloned().unwrap_or_default()).collect();
        cells.push(r.count.to_string());
        cells.push(num(r.volume_m3));
        cells.push(num(r.area_m2));
        cells.push(num(r.length_m));
        if has_cost {
            cells.push(r.cost.as_ref().map(|c| format!("{:.2}", c.amount)).unwrap_or_else(|| "-".into()));
        }
        rows.push(cells);
    }
    let mut total: Vec<String> = vec![String::new(); n_group];
    total[0] = "TOTAL".into();
    total.push(q.total_count.to_string());
    total.push(num(q.total_volume_m3));
    total.push(num(q.total_area_m2));
    total.push(num(q.total_length_m));
    if has_cost {
        total.push(q.total_cost.map(|c| format!("{c:.2}")).unwrap_or_else(|| "-".into()));
    }

    let ncol = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows.iter().chain(std::iter::once(&total)) {
        for (i, c) in row.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let fmt_row = |row: &[String]| -> String {
        let mut line = String::new();
        for i in 0..ncol {
            let c = &row[i];
            let pad = widths[i].saturating_sub(c.chars().count());
            if i < n_group {
                line.push_str(c);
                line.push_str(&" ".repeat(pad));
            } else {
                line.push_str(&" ".repeat(pad));
                line.push_str(c);
            }
            if i + 1 < ncol {
                line.push_str("  ");
            }
        }
        line.trim_end().to_string()
    };
    let mut out = String::new();
    out.push_str("Quantity takeoff\n");
    out.push_str(&fmt_row(&headers));
    out.push('\n');
    out.push_str(&"-".repeat(widths.iter().sum::<usize>() + 2 * (ncol - 1)));
    out.push('\n');
    for r in &rows {
        out.push_str(&fmt_row(r));
        out.push('\n');
    }
    out.push_str(&"-".repeat(widths.iter().sum::<usize>() + 2 * (ncol - 1)));
    out.push('\n');
    out.push_str(&fmt_row(&total));
    out.push('\n');
    if q.elements_without_quantities > 0 {
        out.push_str(&format!("\n{} elements have no usable quantities.\n", q.elements_without_quantities));
    }
    if !q.method.is_empty() {
        out.push_str("\nThe method (how each quantity was taken) is written in report.html and qto.json.\n");
    }
    out
}

fn num(v: f64) -> String {
    format!("{v:.3}")
}

/// 1.2 KB style size.
pub fn human_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    let f = b as f64;
    if f < KB {
        format!("{b} B")
    } else if f < KB * KB {
        format!("{:.1} KB", f / KB)
    } else if f < KB * KB * KB {
        format!("{:.1} MB", f / KB / KB)
    } else {
        format!("{:.2} GB", f / KB / KB / KB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verimodel_core::model::{CostLine, QtoRow};

    #[test]
    fn exit_codes_follow_verdict() {
        assert_eq!(exit_code_for(Some(Verdict::Pass)), 0);
        assert_eq!(exit_code_for(Some(Verdict::PassWithWarnings)), 0);
        assert_eq!(exit_code_for(Some(Verdict::Fail)), 1);
    }

    #[test]
    fn bytes_are_human() {
        assert_eq!(human_bytes(12), "12 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn qto_table_has_columns_totals_and_cost() {
        let q = QtoReport {
            group_by: vec!["type".into(), "storey".into()],
            rows: vec![QtoRow {
                group: vec!["IfcWall".into(), "Level 1".into()],
                count: 3,
                volume_m3: 1.5,
                area_m2: 10.0,
                cost: Some(CostLine { amount: 150.0, ..Default::default() }),
                ..Default::default()
            }],
            total_count: 3,
            total_volume_m3: 1.5,
            total_area_m2: 10.0,
            total_cost: Some(150.0),
            currency: Some("EUR".into()),
            method: "sum of base quantities".into(),
            ..Default::default()
        };
        let t = render_qto(&q);
        assert!(t.contains("type"));
        assert!(t.contains("storey"));
        assert!(t.contains("IfcWall"));
        assert!(t.contains("1.500"));
        assert!(t.contains("cost EUR"));
        assert!(t.contains("TOTAL"));
        assert!(t.contains("method"));
    }

    #[test]
    fn qto_table_without_rows_still_renders() {
        let t = render_qto(&QtoReport::default());
        assert!(t.contains("TOTAL"));
    }

    #[test]
    fn info_lists_types_sorted_by_count() {
        let mut info = ModelInfo { file_name: "a.ifc".into(), schema: "IFC4".into(), element_count: 5, ..Default::default() };
        info.type_counts.insert("IfcDoor".into(), 1);
        info.type_counts.insert("IfcWall".into(), 4);
        let t = render_info(&info);
        assert!(t.find("IfcWall").unwrap() < t.find("IfcDoor").unwrap());
        assert!(t.contains("Schema:      IFC4"));
    }
}
