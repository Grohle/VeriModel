//! Knowledge base and retrieval (RAG without embeddings).
//!
//! A [`Knowledge`] is a list of text chunks in a simple YAML format that a
//! coordinator can read and edit:
//!
//! ```yaml
//! chunks:
//!   - source: docs/RULES.md
//!     title: "Check kinds › property_exists"
//!     text: "Property must exist ..."
//!     keywords: [property, pset]
//! ```
//!
//! Retrieval is BM25 over lower-cased word tokens: deterministic, dependency
//! free and fast enough for a few thousand chunks.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use verimodel_core::model::RunReport;

/// One retrievable unit of text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Chunk {
    pub source: String,
    pub title: String,
    pub text: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// The knowledge base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Knowledge {
    #[serde(default)]
    pub chunks: Vec<Chunk>,
}

/// Built-in documentation embedded at compile time.
const BUILTIN_DOCS: &[(&str, &str)] = &[
    ("docs/CONFIG.md", include_str!("../../../docs/CONFIG.md")),
    ("docs/RULES.md", include_str!("../../../docs/RULES.md")),
    ("docs/IDS.md", include_str!("../../../docs/IDS.md")),
    ("docs/ASSUMPTIONS.md", include_str!("../../../docs/ASSUMPTIONS.md")),
    ("rule_packs/generic.yaml", verimodel_core::rules::pack::GENERIC_RULE_PACK_YAML),
];

pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .map(|t| t.trim_matches(|c| c == '.' || c == '-'))
        .filter(|t| t.len() > 1)
        .map(|t| t.to_string())
        .collect()
}

/// Split markdown into chunks at headings (## / ###), keeping the heading path as title.
pub fn chunk_markdown(source: &str, text: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, path: &Vec<(usize, String)>, out: &mut Vec<Chunk>| {
        let t = buf.trim();
        if !t.is_empty() {
            let title = path.iter().map(|(_, h)| h.as_str()).collect::<Vec<_>>().join(" › ");
            for piece in split_long(t, 1800) {
                out.push(Chunk { source: source.to_string(), title: title.clone(), text: piece, keywords: vec![] });
            }
        }
        buf.clear();
    };
    for line in text.lines() {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes >= 1 && hashes <= 4 && line.chars().nth(hashes) == Some(' ') {
            flush(&mut buf, &path, &mut out);
            path.retain(|(lvl, _)| *lvl < hashes);
            path.push((hashes, line[hashes..].trim().to_string()));
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    flush(&mut buf, &path, &mut out);
    out
}

/// Split text into pieces of at most `max` chars at paragraph boundaries.
pub fn split_long(text: &str, max: usize) -> Vec<String> {
    if text.chars().count() <= max {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n") {
        if !cur.is_empty() && cur.chars().count() + para.chars().count() + 2 > max {
            out.push(cur.trim().to_string());
            cur.clear();
        }
        if para.chars().count() > max {
            for (i, ch) in para.chars().enumerate() {
                if i > 0 && i % max == 0 {
                    out.push(cur.trim().to_string());
                    cur.clear();
                }
                cur.push(ch);
            }
            cur.push_str("\n\n");
        } else {
            cur.push_str(para);
            cur.push_str("\n\n");
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

impl Knowledge {
    /// The built-in docs and the generic rule pack, chunked.
    pub fn builtin() -> Knowledge {
        let mut k = Knowledge::default();
        for (src, text) in BUILTIN_DOCS {
            if src.ends_with(".yaml") {
                k.chunks.extend(chunk_rule_pack(src, text));
            } else {
                k.chunks.extend(chunk_markdown(src, text));
            }
        }
        k
    }

    pub fn from_yaml(text: &str) -> Result<Knowledge, String> {
        serde_yaml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(self).unwrap_or_default()
    }

    pub fn extend(&mut self, other: Knowledge) {
        self.chunks.extend(other.chunks);
    }

    /// Add the current report as knowledge: one chunk per rule (with its outcome)
    /// and summary chunks for the model, storeys, types and QTO totals.
    pub fn add_report(&mut self, report: &RunReport) {
        let src = format!("report:{}", report.model.file_name);
        let s = &report.summary;
        self.chunks.push(Chunk {
            source: src.clone(),
            title: "Run summary".into(),
            text: format!(
                "File {} ({}), {} elements. Verdict {:?}, score {}. Errors {}, warnings {}, info {}. Rules: {} passed, {} failed, {} skipped, {} not applicable. Storeys: {}. Disciplines: {}. Element types: {}.",
                report.model.file_name,
                report.model.schema,
                report.model.element_count,
                s.verdict,
                s.score,
                s.errors,
                s.warnings,
                s.infos,
                s.rules_passed,
                s.rules_failed,
                s.rules_skipped,
                s.rules_not_applicable,
                report.model.storeys.join(", "),
                report.model.disciplines.join(", "),
                report.model.type_counts.iter().map(|(t, n)| format!("{t} ({n})")).collect::<Vec<_>>().join(", ")
            ),
            keywords: vec!["summary".into(), "verdict".into(), "score".into()],
        });
        for r in &report.rules {
            let mut text = format!(
                "Rule {} [{}] — {}. Severity {}. Status {:?}. Elements checked {}, failures {}.",
                r.rule_id, r.category, r.description, r.severity.as_str(), r.status, r.elements_checked, r.failures
            );
            if let Some(reason) = &r.skipped_reason {
                text.push_str(&format!(" Skipped because: {reason}."));
            }
            let examples: Vec<String> = report
                .findings
                .iter()
                .filter(|f| f.rule_id == r.rule_id)
                .take(5)
                .map(|f| format!("{} {} ({}) in {}: {}", f.ifc_type.clone().unwrap_or_default(), f.name.clone().unwrap_or_default(), f.global_id.clone().unwrap_or_default(), f.storey.clone().unwrap_or_else(|| "-".into()), f.message))
                .collect();
            if !examples.is_empty() {
                text.push_str(" Examples: ");
                text.push_str(&examples.join(" | "));
            }
            self.chunks.push(Chunk { source: src.clone(), title: format!("Rule {}", r.rule_id), text, keywords: vec![r.rule_id.to_lowercase(), r.category.clone()] });
        }
        if !report.qto.rows.is_empty() {
            let rows: Vec<String> = report
                .qto
                .rows
                .iter()
                .take(60)
                .map(|r| format!("{}: count {}, volume {:.3} m3, area {:.3} m2, length {:.3} m", r.group.join(" / "), r.count, r.volume_m3, r.area_m2, r.length_m))
                .collect();
            self.chunks.push(Chunk {
                source: src,
                title: "Quantities".into(),
                text: format!("Grouped by {}. {} Totals: count {}, volume {:.3} m3, area {:.3} m2, length {:.3} m.", report.qto.group_by.join(", "), rows.join("; "), report.qto.total_count, report.qto.total_volume_m3, report.qto.total_area_m2, report.qto.total_length_m),
                keywords: vec!["qto".into(), "quantities".into(), "volume".into(), "area".into()],
            });
        }
    }

    /// Top-k chunks for a query by BM25 (k1 = 1.5, b = 0.75). Deterministic ties by index.
    pub fn search(&self, query: &str, k: usize) -> Vec<(&Chunk, f64)> {
        let q: Vec<String> = tokenize(query);
        if q.is_empty() || self.chunks.is_empty() {
            return vec![];
        }
        let docs: Vec<Vec<String>> = self
            .chunks
            .iter()
            .map(|c| {
                let mut t = tokenize(&c.title);
                t.extend(tokenize(&c.text));
                t.extend(c.keywords.iter().map(|k| k.to_lowercase()));
                t
            })
            .collect();
        let n = docs.len() as f64;
        let avg = docs.iter().map(|d| d.len()).sum::<usize>() as f64 / n;
        let mut df: HashMap<&str, usize> = HashMap::new();
        for d in &docs {
            let mut seen = BTreeMap::new();
            for t in d {
                seen.insert(t.as_str(), ());
            }
            for t in seen.keys() {
                *df.entry(t).or_default() += 1;
            }
        }
        let (k1, b) = (1.5, 0.75);
        let mut scored: Vec<(usize, f64)> = docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let len = d.len() as f64;
                let mut tf: HashMap<&str, f64> = HashMap::new();
                for t in d {
                    *tf.entry(t.as_str()).or_default() += 1.0;
                }
                let mut score = 0.0;
                for term in &q {
                    let f = *tf.get(term.as_str()).unwrap_or(&0.0);
                    if f == 0.0 {
                        continue;
                    }
                    let dfi = *df.get(term.as_str()).unwrap_or(&0) as f64;
                    let idf = ((n - dfi + 0.5) / (dfi + 0.5) + 1.0).ln();
                    score += idf * (f * (k1 + 1.0)) / (f + k1 * (1.0 - b + b * len / avg));
                }
                (i, score)
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
        scored.into_iter().take(k).map(|(i, s)| (&self.chunks[i], s)).collect()
    }
}

/// One chunk per rule of a YAML rule pack.
pub fn chunk_rule_pack(source: &str, text: &str) -> Vec<Chunk> {
    let Ok(pack) = verimodel_core::rules::RulePack::from_yaml(text) else { return vec![] };
    pack.rules
        .iter()
        .map(|r| Chunk {
            source: source.to_string(),
            title: format!("Rule {} ({})", r.id, r.category),
            text: format!(
                "{} — {}. Severity {}. Check kind: {}. Scope types: {}. Fix hint: {}",
                r.id,
                r.description,
                r.severity,
                r.check.kind_name(),
                if r.scope.ifc_types.is_empty() { "all".to_string() } else { r.scope.ifc_types.join(", ") },
                r.fix_hint.clone().unwrap_or_default()
            ),
            keywords: vec![r.id.to_lowercase(), r.category.clone(), r.check.kind_name().to_string()],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_knowledge_is_searchable() {
        let k = Knowledge::builtin();
        assert!(k.chunks.len() > 40, "{}", k.chunks.len());
        let hits = k.search("duplicate GlobalId", 3);
        assert!(!hits.is_empty());
        assert!(hits[0].0.text.to_lowercase().contains("globalid"), "{}", hits[0].0.text);
        let hits = k.search("far from origin tolerance", 3);
        assert!(hits.iter().any(|(c, _)| c.text.contains("far_from_origin_m")));
    }

    #[test]
    fn yaml_roundtrip() {
        let k = Knowledge { chunks: vec![Chunk { source: "a".into(), title: "t".into(), text: "hello world".into(), keywords: vec!["x".into()] }] };
        let y = k.to_yaml();
        assert_eq!(Knowledge::from_yaml(&y).unwrap(), k);
    }

    #[test]
    fn markdown_chunking_keeps_heading_path() {
        let c = chunk_markdown("f.md", "# A\nintro\n## B\ntext b\n### C\ntext c\n## D\ntext d\n");
        let titles: Vec<_> = c.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["A", "A › B", "A › B › C", "A › D"]);
    }
}
