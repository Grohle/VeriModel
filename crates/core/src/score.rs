//! Score and verdict. The formula is deliberately simple and stated in the UI:
//!
//! * **Verdict**: FAIL if any ERROR finding; PASS WITH WARNINGS if no errors
//!   but at least one WARNING; PASS otherwise. Skipped rules never count as
//!   passed and are listed separately.
//! * **Score** (0–100): `100 × (1 − (errors + 0.25 × warnings) / element_checks)`,
//!   where `element_checks` is the number of (element, rule) pairs evaluated.
//!   INFO findings do not affect the score. Clamped to 0..=100.

use crate::model::{Finding, RuleResult, RuleStatus, Severity, Summary, Verdict};

pub const WARNING_WEIGHT: f64 = 0.25;

/// Plain-language statement of the verdict rule, shown in the UI and report.
pub const VERDICT_RULE: &str = "FAIL = at least one ERROR. PASS WITH WARNINGS = no errors, at least one WARNING. PASS = no errors and no warnings. Score = 100 × (1 − (errors + 0.25 × warnings) / element checks). Skipped rules are never counted as passed.";

pub fn summarise(rules: &[RuleResult], findings: &[Finding]) -> Summary {
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .count();
    let infos = findings
        .iter()
        .filter(|f| f.severity == Severity::Info)
        .count();
    let element_checks: usize = rules
        .iter()
        .filter(|r| r.status != RuleStatus::Skipped)
        .map(|r| r.elements_checked)
        .sum();
    let mut distinct = std::collections::BTreeSet::new();
    for f in findings {
        if let Some(g) = &f.global_id {
            distinct.insert(g.clone());
        }
    }
    let score = if element_checks == 0 {
        if errors > 0 {
            0
        } else {
            100
        }
    } else {
        let penalty = errors as f64 + WARNING_WEIGHT * warnings as f64;
        let s = 100.0 * (1.0 - penalty / element_checks as f64);
        s.round().clamp(0.0, 100.0) as u8
    };
    let verdict = if rules.is_empty() {
        None
    } else if errors > 0 {
        Some(Verdict::Fail)
    } else if warnings > 0 {
        Some(Verdict::PassWithWarnings)
    } else {
        Some(Verdict::Pass)
    };
    Summary {
        score,
        verdict,
        elements_checked: rules.iter().map(|r| r.elements_checked).max().unwrap_or(0),
        element_checks,
        errors,
        warnings,
        infos,
        rules_total: rules.len(),
        rules_passed: rules
            .iter()
            .filter(|r| r.status == RuleStatus::Pass)
            .count(),
        rules_failed: rules
            .iter()
            .filter(|r| r.status == RuleStatus::Fail)
            .count(),
        rules_skipped: rules
            .iter()
            .filter(|r| r.status == RuleStatus::Skipped)
            .count(),
        rules_not_applicable: rules
            .iter()
            .filter(|r| r.status == RuleStatus::NotApplicable)
            .count(),
        verdict_rule: VERDICT_RULE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RuleSource;

    fn rule(id: &str, status: RuleStatus, checked: usize, failures: usize) -> RuleResult {
        RuleResult {
            rule_id: id.into(),
            category: "x".into(),
            description: "d".into(),
            severity: Severity::Error,
            status,
            elements_checked: checked,
            failures,
            skipped_reason: None,
            source: RuleSource::ProjectConfig,
        }
    }
    fn finding(sev: Severity) -> Finding {
        Finding {
            rule_id: "r".into(),
            severity: sev,
            global_id: Some("g".into()),
            step_id: None,
            ifc_type: None,
            name: None,
            storey: None,
            message: "m".into(),
            expected: None,
            actual: None,
            fix_hint: None,
            source: RuleSource::ProjectConfig,
        }
    }

    #[test]
    fn score_formula() {
        let rules = vec![
            rule("a", RuleStatus::Fail, 100, 10),
            rule("b", RuleStatus::Pass, 100, 0),
        ];
        let findings: Vec<Finding> = (0..10).map(|_| finding(Severity::Error)).collect();
        let s = summarise(&rules, &findings);
        assert_eq!(s.score, 95);
        assert_eq!(s.verdict, Some(Verdict::Fail));
        let warn: Vec<Finding> = (0..8).map(|_| finding(Severity::Warning)).collect();
        let s2 = summarise(&rules, &warn);
        assert_eq!(s2.score, 99);
        assert_eq!(s2.verdict, Some(Verdict::PassWithWarnings));
    }

    #[test]
    fn skipped_rules_do_not_count() {
        let rules = vec![rule("a", RuleStatus::Skipped, 0, 0)];
        let s = summarise(&rules, &[]);
        assert_eq!(s.rules_skipped, 1);
        assert_eq!(s.rules_passed, 0);
        assert_eq!(s.score, 100);
        assert_eq!(s.verdict, Some(Verdict::Pass));
    }
}
