//! Tasks: what the assistant can be asked, how it is prompted, and how its
//! answer is validated before anyone sees it.
//!
//! Contract:
//! * Build messages: a system prompt stating the role and the JSON schema of the
//!   expected answer, then up to `k` retrieved chunks (BM25 over the knowledge),
//!   then the user content. Never include anything the user did not provide or
//!   that is not in the knowledge base.
//! * Generate with `GenOptions::default()` (greedy), parse the first JSON
//!   object in the output, deserialise into the task's answer type, validate
//!   (see each variant), and on failure retry ONCE with the parser error
//!   appended; if it still fails return `Answer::Text { text, structured: false }`.
//! * `Task::Filter`: answer `FilterSpec`; every `rule_id`, `storey`, `ifc_type`,
//!   `severity` must be in `ReportContext` (case-insensitive match → canonical
//!   value) else dropped and listed in `dropped`.
//! * `Task::Explain`: answer text (2–5 sentences, plain language, mentions the
//!   element name/GlobalId and the fix). Input facts are the finding + rule.
//! * `Task::ConfigHelp`: answer `{ yaml, explanation }`; `yaml` is validated
//!   with `verimodel_core::Config::from_yaml` → `valid`, `validation_error`.
//! * `Task::DraftRules`: requirement text (e.g. from converted documentation)
//!   → a `required_psets` / `naming` / `classification` YAML fragment, validated
//!   the same way.
//! * `Task::Ask`: free question → `Answer::Text` with the titles of the chunks
//!   used as `sources`.
//!
//! Prompt templates live in `crates/assistant/prompts/*.txt` (data, not code).
//! A template starts with a `[system]` section (task instructions + schema)
//! and may continue with `[user]` / `[assistant]` example turns (few-shot).
//! For `ConfigHelp` / `DraftRules`, YAML that still does not validate after
//! the retry is returned as `ConfigSnippet { valid: false, .. }` (flagged,
//! never applied); only output that is not JSON at all becomes plain text.
//!
//! Implementation notes (all deterministic):
//! * The prompt ends with the opening of the answer object (`{"answer":`),
//!   which keeps a 0.5B model in JSON; the prefix is re-attached before parsing.
//! * Invalid YAML (and a non-matching `ConfigHelp`/`DraftRules` answer) also
//!   triggers the single retry, with the validation error in the message.
//! * Literal newlines inside JSON strings are repaired; for every task except
//!   `Filter`, trailing junk after complete leading fields is ignored
//!   (`{"answer": "...", "id": 001}`). Filters are never salvaged.
//! * `Filter` retrieves `RETRIEVE_K / 2` chunks (the allowed values are its
//!   real context); a missing `Ifc` prefix and plurals ("errors") are
//!   canonicalised to an existing value; `dropped` entries read `field: value`.
//! * `ConfigHelp`/`DraftRules` fragments (e.g. a bare `required_psets:`) are
//!   wrapped into their `project.yaml` section before validation and returned
//!   wrapped, so the snippet can be pasted as is.

use serde::{Deserialize, Serialize};

use verimodel_core::model::{Finding, RuleResult};

use crate::backend::{chatml, Backend, GenOptions, Message};
use crate::error::AssistantResult;
use crate::knowledge::{Chunk, Knowledge};

/// Allowed values for filter answers: taken from the current report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReportContext {
    pub rule_ids: Vec<String>,
    pub storeys: Vec<String>,
    pub ifc_types: Vec<String>,
    /// "ERROR" | "WARNING" | "INFO"
    pub severities: Vec<String>,
}

/// A UI filter the assistant proposes; the UI applies it only after the user confirms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilterSpec {
    #[serde(default)]
    pub severity: Vec<String>,
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub storey: Option<String>,
    #[serde(default)]
    pub ifc_type: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub explanation: String,
    /// Values the model proposed that do not exist in the report (dropped).
    #[serde(default)]
    pub dropped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Task {
    Filter {
        question: String,
        context: ReportContext,
    },
    Explain {
        finding: Box<Finding>,
        rule: Option<RuleResult>,
    },
    ConfigHelp {
        question: String,
        current_yaml: Option<String>,
    },
    DraftRules {
        requirements_text: String,
    },
    Ask {
        question: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Filter {
        filter: FilterSpec,
    },
    Explanation {
        text: String,
    },
    ConfigSnippet {
        yaml: String,
        valid: bool,
        validation_error: Option<String>,
        explanation: String,
    },
    Text {
        text: String,
        sources: Vec<String>,
        structured: bool,
    },
}

/// Number of retrieved chunks put in the prompt.
pub const RETRIEVE_K: usize = 4;

/// Maximum characters of one retrieved chunk put in the prompt (keeps prefill fast).
const CHUNK_PROMPT_CHARS: usize = 1200;
/// Maximum characters of user-provided long text (requirements, current YAML).
const LONG_TEXT_CHARS: usize = 4000;

const PROMPT_SYSTEM: &str = include_str!("../prompts/system.txt");
const PROMPT_FILTER: &str = include_str!("../prompts/filter.txt");
const PROMPT_EXPLAIN: &str = include_str!("../prompts/explain.txt");
const PROMPT_CONFIG_HELP: &str = include_str!("../prompts/config_help.txt");
const PROMPT_DRAFT_RULES: &str = include_str!("../prompts/draft_rules.txt");
const PROMPT_ASK: &str = include_str!("../prompts/ask.txt");

const RETRY_MESSAGE: &str = "Format error in your reply ({error}). Write your answer again as ONLY one JSON object with this schema: {schema}";

/// Run a task against a backend with the given knowledge.
pub fn run(
    backend: &mut dyn Backend,
    knowledge: &Knowledge,
    task: &Task,
) -> AssistantResult<Answer> {
    // A filter is grounded in the allowed values; extra documentation only adds noise and latency.
    let k = if matches!(task, Task::Filter { .. }) {
        RETRIEVE_K / 2
    } else {
        RETRIEVE_K
    };
    let hits = knowledge.search(&retrieval_query(task), k);
    let chunks: Vec<&Chunk> = hits.iter().map(|(c, _)| *c).collect();
    let sources = source_titles(&chunks);
    let mut messages = build_messages(task, &chunks);
    let opts = GenOptions::default();

    // The answer is primed with the opening of the JSON object ({"answer":),
    // which keeps small models in the required format.
    let prefix = json_prefix(task);
    let mut raw = with_prefix(
        &prefix,
        &backend.generate(&format!("{}{prefix}", chatml(&messages)), &opts)?,
    );
    let mut parsed = parse_answer(task, &raw);
    if let Err(error) = &parsed {
        log::debug!("assistant answer rejected ({error}), retrying once");
        messages.push(Message {
            role: "assistant",
            content: raw.trim().to_string(),
        });
        let retry = RETRY_MESSAGE
            .replace("{error}", error)
            .replace("{schema}", &schema_line(task));
        messages.push(Message {
            role: "user",
            content: retry,
        });
        raw = with_prefix(
            &prefix,
            &backend.generate(&format!("{}{prefix}", chatml(&messages)), &opts)?,
        );
        parsed = parse_answer(task, &raw);
    }
    Ok(match parsed {
        Ok(mut answer) => {
            if let Answer::Text { sources: s, .. } = &mut answer {
                *s = sources;
            }
            answer
        }
        // The YAML still does not validate after the retry: show it, flagged.
        Err(_) if matches!(task, Task::ConfigHelp { .. } | Task::DraftRules { .. }) => {
            match parse_snippet(&raw) {
                Some(answer) => answer,
                None => Answer::Text {
                    text: plain_text(&prefix, &raw),
                    sources,
                    structured: false,
                },
            }
        }
        Err(_) => Answer::Text {
            text: plain_text(&prefix, &raw),
            sources,
            structured: false,
        },
    })
}

// ---------------------------------------------------------------- prompts

fn retrieval_query(task: &Task) -> String {
    match task {
        Task::Filter { question, .. } => question.clone(),
        Task::Explain { finding, rule } => {
            let mut q = format!(
                "{} {} {}",
                finding.rule_id,
                finding.message,
                finding.fix_hint.clone().unwrap_or_default()
            );
            if let Some(r) = rule {
                q.push(' ');
                q.push_str(&r.description);
            }
            q
        }
        Task::ConfigHelp { question, .. } => format!("project.yaml {question}"),
        Task::DraftRules { requirements_text } => format!(
            "required_psets naming patterns classification {}",
            truncate_chars(requirements_text, 600)
        ),
        Task::Ask { question } => question.clone(),
    }
}

fn source_titles(chunks: &[&Chunk]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in chunks {
        let t = if c.title.trim().is_empty() {
            c.source.clone()
        } else {
            c.title.clone()
        };
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// The one-line JSON schema of a task (the line after "JSON schema:" in its template).
fn schema_line(task: &Task) -> String {
    let (system, _) = parse_template(template(task));
    let mut lines = system.lines();
    lines.by_ref().find(|l| l.trim() == "JSON schema:");
    lines.next().unwrap_or_default().trim().to_string()
}

/// `{"<first schema key>":`, appended after the assistant header of the prompt.
fn json_prefix(task: &Task) -> String {
    let schema = schema_line(task);
    match schema.split('"').nth(1) {
        Some(key) if !key.is_empty() => format!("{{\"{key}\":"),
        _ => "{".into(),
    }
}

/// Re-attach the primed prefix unless the model restarted the object itself.
fn with_prefix(prefix: &str, output: &str) -> String {
    let t = output.trim_start();
    let unfenced = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    if unfenced.trim_start().starts_with('{') {
        output.to_string()
    } else {
        format!("{prefix}{output}")
    }
}

/// Unparseable output shown as plain text, without the primed JSON prefix.
fn plain_text(prefix: &str, raw: &str) -> String {
    match raw.strip_prefix(prefix) {
        Some(rest) => rest
            .trim()
            .trim_start_matches('"')
            .trim_end_matches('}')
            .trim_end()
            .trim_end_matches('"')
            .trim()
            .to_string(),
        None => raw.trim().to_string(),
    }
}

fn template(task: &Task) -> &'static str {
    match task {
        Task::Filter { .. } => PROMPT_FILTER,
        Task::Explain { .. } => PROMPT_EXPLAIN,
        Task::ConfigHelp { .. } => PROMPT_CONFIG_HELP,
        Task::DraftRules { .. } => PROMPT_DRAFT_RULES,
        Task::Ask { .. } => PROMPT_ASK,
    }
}

/// Split a template into its `[system]` text and example turns.
fn parse_template(text: &str) -> (String, Vec<Message>) {
    let mut system = String::new();
    let mut turns: Vec<Message> = Vec::new();
    let mut current: Option<&'static str> = None;
    let mut buf = String::new();
    let flush = |role: Option<&'static str>,
                 buf: &mut String,
                 system: &mut String,
                 turns: &mut Vec<Message>| {
        let content = buf.trim().to_string();
        buf.clear();
        match role {
            Some("system") => *system = content,
            Some(r) if !content.is_empty() => turns.push(Message { role: r, content }),
            _ => {}
        }
    };
    for line in text.lines() {
        let role = match line.trim() {
            "[system]" => Some("system"),
            "[user]" => Some("user"),
            "[assistant]" => Some("assistant"),
            _ => None,
        };
        if let Some(r) = role {
            flush(current, &mut buf, &mut system, &mut turns);
            current = Some(r);
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    flush(current, &mut buf, &mut system, &mut turns);
    (system, turns)
}

fn context_block(chunks: &[&Chunk]) -> String {
    if chunks.is_empty() {
        return "Context: (nothing relevant found)\n\n".into();
    }
    let mut s = String::from("Context:\n");
    for (i, c) in chunks.iter().enumerate() {
        s.push_str(&format!(
            "[{}] {} (source: {})\n{}\n\n",
            i + 1,
            c.title,
            c.source,
            truncate_chars(c.text.trim(), CHUNK_PROMPT_CHARS)
        ));
    }
    s
}

fn user_content(task: &Task) -> String {
    match task {
        Task::Filter { question, context } => format!(
            "Allowed values:\nrule_id: {}\nstorey: {}\nifc_type: {}\nseverity: {}\n\nRequest: {}",
            context.rule_ids.join(", "),
            context.storeys.join(", "),
            context.ifc_types.join(", "),
            context.severities.join(", "),
            question.trim()
        ),
        Task::Explain { finding, rule } => {
            let mut s = format!(
                "Finding:\n- rule: {} (severity {})\n",
                finding.rule_id,
                finding.severity.as_str()
            );
            let mut element = String::from("- element: ");
            element.push_str(finding.ifc_type.as_deref().unwrap_or("model-level finding"));
            if let Some(n) = &finding.name {
                element.push_str(&format!(" \"{n}\""));
            }
            if let Some(g) = &finding.global_id {
                element.push_str(&format!(" (GlobalId {g})"));
            }
            if let Some(st) = &finding.storey {
                element.push_str(&format!(", storey {st}"));
            }
            s.push_str(&element);
            s.push('\n');
            s.push_str(&format!("- message: {}\n", finding.message));
            for (label, v) in [
                ("expected", &finding.expected),
                ("actual", &finding.actual),
                ("fix hint", &finding.fix_hint),
            ] {
                if let Some(v) = v {
                    s.push_str(&format!("- {label}: {v}\n"));
                }
            }
            if let Some(r) = rule {
                s.push_str(&format!(
                    "\nRule: {} [{}] {}. Status {:?}, {} failures out of {} elements.",
                    r.rule_id, r.category, r.description, r.status, r.failures, r.elements_checked
                ));
            }
            s
        }
        Task::ConfigHelp {
            question,
            current_yaml,
        } => {
            let mut s = String::new();
            if let Some(y) = current_yaml.as_deref().filter(|y| !y.trim().is_empty()) {
                s.push_str(&format!(
                    "Current project.yaml:\n{}\n\n",
                    truncate_chars(y.trim(), LONG_TEXT_CHARS)
                ));
            }
            s.push_str(&format!("Question: {}", question.trim()));
            s
        }
        Task::DraftRules { requirements_text } => format!(
            "Requirements text:\n{}",
            truncate_chars(requirements_text.trim(), LONG_TEXT_CHARS)
        ),
        Task::Ask { question } => format!("Question: {}", question.trim()),
    }
}

/// System prompt + schema, few-shot examples, then context + user content.
fn build_messages(task: &Task, chunks: &[&Chunk]) -> Vec<Message> {
    let (task_system, examples) = parse_template(template(task));
    let mut messages = vec![Message {
        role: "system",
        content: format!("{}\n\n{}", PROMPT_SYSTEM.trim(), task_system),
    }];
    messages.extend(examples);
    messages.push(Message {
        role: "user",
        content: format!("{}{}", context_block(chunks), user_content(task)),
    });
    messages
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

// ---------------------------------------------------------------- parsing

/// The first balanced `{...}` in `text` (markdown fences ignored, strings respected).
pub fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let bytes = text.as_bytes();
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            match b {
                _ if esc => esc = false,
                b'\\' => esc = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Escape raw control characters inside JSON strings (small models often put
/// literal newlines in a "yaml" value).
fn repair_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let (mut in_str, mut esc) = (false, false);
    for ch in s.chars() {
        if in_str {
            if esc {
                esc = false;
                out.push(ch);
                continue;
            }
            match ch {
                '\\' => {
                    esc = true;
                    out.push(ch);
                }
                '"' => {
                    in_str = false;
                    out.push(ch);
                }
                '\n' => out.push_str("\\n"),
                '\r' => {}
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {}
                c => out.push(c),
            }
        } else {
            if ch == '"' {
                in_str = true;
            }
            out.push(ch);
        }
    }
    out
}

/// Parse the first JSON object of a model output.
pub fn parse_json_object(
    output: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let obj =
        extract_json_object(output).ok_or_else(|| "no complete JSON object found".to_string())?;
    let value: serde_json::Value = serde_json::from_str(obj)
        .or_else(|_| serde_json::from_str(&repair_json(obj)))
        .map_err(|e| format!("invalid JSON: {e}"))?;
    match value {
        serde_json::Value::Object(m) => Ok(m),
        _ => Err("the answer is not a JSON object".into()),
    }
}

/// Leading complete `"key": value` pairs of a malformed object, e.g.
/// `{"answer": "text", "id": 001}` → `{"answer": "text"}`. Used only for
/// tasks whose essential field comes first (never for filters, where a lost
/// field would silently widen the result).
fn salvage_leading_pairs(output: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let start = output.find('{')?;
    let repaired = repair_json(&output[start + 1..]);
    let mut rest = repaired.as_str();
    let mut map = serde_json::Map::new();
    loop {
        rest = rest.trim_start();
        if !rest.starts_with('"') {
            break;
        }
        let mut keys = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        let Some(Ok(key)) = keys.next() else { break };
        rest = rest[keys.byte_offset()..].trim_start();
        let Some(after_colon) = rest.strip_prefix(':') else {
            break;
        };
        let mut values =
            serde_json::Deserializer::from_str(after_colon).into_iter::<serde_json::Value>();
        let Some(Ok(value)) = values.next() else {
            break;
        };
        rest = &after_colon[values.byte_offset()..];
        // A value is complete only if a separator follows it.
        let next = rest.trim_start();
        if !(next.starts_with(',') || next.starts_with('}')) {
            break;
        }
        map.insert(key, value);
        match next.strip_prefix(',') {
            Some(r) => rest = r,
            None => break,
        }
    }
    (!map.is_empty()).then_some(map)
}

/// A string field; JSON null, missing, empty and "null"/"none" are `None`.
fn opt_str(m: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match m.get(key)? {
        serde_json::Value::String(s) => {
            let t = s.trim();
            if t.is_empty()
                || ["null", "none", "any", "all", "n/a"].contains(&t.to_lowercase().as_str())
            {
                None
            } else {
                Some(t.to_string())
            }
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn first_str(m: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| opt_str(m, k))
}

fn parse_answer(task: &Task, output: &str) -> Result<Answer, String> {
    let m = match parse_json_object(output) {
        Ok(m) => m,
        Err(e) if matches!(task, Task::Filter { .. }) => return Err(e),
        Err(e) => salvage_leading_pairs(output).ok_or(e)?,
    };
    match task {
        Task::Filter { context, .. } => Ok(Answer::Filter {
            filter: validate_filter(&m, context)?,
        }),
        Task::Explain { .. } => {
            let text = first_str(&m, &["text", "explanation", "answer"])
                .ok_or_else(|| "missing \"text\"".to_string())?;
            Ok(Answer::Explanation { text })
        }
        Task::Ask { .. } => {
            let text = first_str(&m, &["answer", "text"])
                .ok_or_else(|| "missing \"answer\"".to_string())?;
            Ok(Answer::Text {
                text,
                sources: vec![],
                structured: true,
            })
        }
        Task::ConfigHelp { .. } | Task::DraftRules { .. } => {
            let answer = snippet_from(&m)?;
            match &answer {
                Answer::ConfigSnippet {
                    valid: false,
                    validation_error,
                    ..
                } => Err(format!(
                    "the YAML is not a valid VeriModel configuration: {}",
                    validation_error.clone().unwrap_or_default()
                )),
                _ => Ok(answer),
            }
        }
    }
}

// ---------------------------------------------------------------- filter

/// Canonical value from `allowed` (case-insensitive; plural "errors" and a
/// missing "Ifc" prefix are tolerated).
fn canonical(value: &str, allowed: &[String]) -> Option<String> {
    let v = value.trim().to_lowercase();
    let find = |cand: &str| {
        allowed
            .iter()
            .find(|a| a.trim().to_lowercase() == cand)
            .cloned()
    };
    find(&v)
        .or_else(|| v.strip_suffix('s').and_then(find))
        .or_else(|| {
            if v.starts_with("ifc") {
                None
            } else {
                find(&format!("ifc{v}"))
            }
        })
}

fn validate_filter(
    m: &serde_json::Map<String, serde_json::Value>,
    ctx: &ReportContext,
) -> Result<FilterSpec, String> {
    let known = [
        "severity",
        "rule_id",
        "storey",
        "ifc_type",
        "search",
        "explanation",
    ];
    if !m.keys().any(|k| known.contains(&k.as_str())) {
        return Err(format!("expected the keys {}", known.join(", ")));
    }
    let mut f = FilterSpec {
        explanation: opt_str(m, "explanation").unwrap_or_default(),
        search: opt_str(m, "search"),
        ..FilterSpec::default()
    };
    let severities: Vec<String> = match m.get("severity") {
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => s.split(',').map(str::to_string).collect(),
        _ => vec![],
    };
    for s in severities
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        match canonical(s, &ctx.severities) {
            Some(c) if !f.severity.contains(&c) => f.severity.push(c),
            Some(_) => {}
            None => f.dropped.push(format!("severity: {s}")),
        }
    }
    let mut field = |key: &str, allowed: &[String]| -> Option<String> {
        let v = opt_str(m, key)?;
        let c = canonical(&v, allowed);
        if c.is_none() {
            f.dropped.push(format!("{key}: {v}"));
        }
        c
    };
    let rule_id = field("rule_id", &ctx.rule_ids);
    let storey = field("storey", &ctx.storeys);
    let ifc_type = field("ifc_type", &ctx.ifc_types);
    f.rule_id = rule_id;
    f.storey = storey;
    f.ifc_type = ifc_type;
    Ok(f)
}

// ---------------------------------------------------------------- config snippets

/// Top-level keys of `project.yaml`.
const CONFIG_SECTIONS: &[&str] = &[
    "project",
    "requirements",
    "tolerances",
    "qto",
    "clash",
    "output",
    "checks",
    "naming",
    "classification",
];

/// Section that owns a key the model may have written without its parent.
fn section_of(key: &str) -> Option<&'static str> {
    Some(match key {
        "required_psets" | "ids_file" | "rule_pack" => "requirements",
        "patterns" => "naming",
        "required" | "applies_to" | "allowed_codes" | "allowed_codes_file" | "code_pattern" => {
            "classification"
        }
        "rules" | "categories" | "unique_name_within" => "checks",
        "clash_hard_mm" | "clash_clearance_mm" | "far_from_origin_m" | "geometry" => "tolerances",
        "group_by" | "unit_rates_file" | "currency" | "sample_count" => "qto",
        _ => return None,
    })
}

fn strip_fences(s: &str) -> String {
    let t = s.trim();
    if !t.starts_with("```") {
        return t.to_string();
    }
    let body = t.split_once('\n').map(|(_, b)| b).unwrap_or("");
    body.trim_end()
        .trim_end_matches("```")
        .trim_end()
        .to_string()
}

/// Wrap a fragment (e.g. a bare `required_psets:` list) into full
/// `project.yaml` sections, then validate with `Config::from_yaml`.
/// Returns (normalised YAML, validation error).
pub fn normalize_and_validate_yaml(yaml: &str) -> (String, Option<String>) {
    let text = strip_fences(yaml);
    if text.trim().is_empty() {
        return (text, Some("the YAML is empty".into()));
    }
    let value: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(v) => v,
        Err(e) => return (text, Some(format!("YAML syntax error: {e}"))),
    };
    let mut text = text;
    if let serde_yaml::Value::Mapping(map) = &value {
        let keys: Vec<String> = map
            .keys()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect();
        let needs_wrap = keys
            .iter()
            .any(|k| !CONFIG_SECTIONS.contains(&k.as_str()) && section_of(k).is_some());
        let all_known = keys
            .iter()
            .all(|k| CONFIG_SECTIONS.contains(&k.as_str()) || section_of(k).is_some());
        if needs_wrap && all_known {
            let mut out = serde_yaml::Mapping::new();
            for (k, v) in map {
                let key = k.as_str().unwrap_or_default();
                let (section, inner): (&str, Option<&str>) = if CONFIG_SECTIONS.contains(&key) {
                    (key, None)
                } else {
                    (section_of(key).unwrap_or(key), Some(key))
                };
                let skey = serde_yaml::Value::String(section.to_string());
                match inner {
                    None => match (out.get_mut(&skey), v) {
                        (
                            Some(serde_yaml::Value::Mapping(existing)),
                            serde_yaml::Value::Mapping(new),
                        ) => {
                            for (nk, nv) in new {
                                existing.insert(nk.clone(), nv.clone());
                            }
                        }
                        _ => {
                            out.insert(skey, v.clone());
                        }
                    },
                    Some(inner) => {
                        let entry = out.entry(skey).or_insert_with(|| {
                            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
                        });
                        if let serde_yaml::Value::Mapping(m) = entry {
                            m.insert(serde_yaml::Value::String(inner.to_string()), v.clone());
                        }
                    }
                }
            }
            if let Ok(s) = serde_yaml::to_string(&serde_yaml::Value::Mapping(out)) {
                text = s;
            }
        }
    }
    let err = verimodel_core::config::Config::from_yaml(&text)
        .err()
        .map(|e| e.to_string());
    (text.trim_end().to_string() + "\n", err)
}

fn snippet_from(m: &serde_json::Map<String, serde_json::Value>) -> Result<Answer, String> {
    let yaml = match m.get("yaml") {
        Some(serde_json::Value::String(s)) => s.clone(),
        // A model may emit the YAML as a nested object: render it.
        Some(v @ serde_json::Value::Object(_)) => {
            serde_yaml::to_string(v).map_err(|e| e.to_string())?
        }
        _ => return Err("missing \"yaml\"".into()),
    };
    let explanation = opt_str(m, "explanation").unwrap_or_default();
    let (yaml, validation_error) = normalize_and_validate_yaml(&yaml);
    Ok(Answer::ConfigSnippet {
        yaml,
        valid: validation_error.is_none(),
        validation_error,
        explanation,
    })
}

fn parse_snippet(output: &str) -> Option<Answer> {
    parse_json_object(output)
        .ok()
        .and_then(|m| snippet_from(&m).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;
    use verimodel_core::model::{RuleSource, RuleStatus, Severity};

    fn ctx() -> ReportContext {
        ReportContext {
            rule_ids: vec!["ID-001".into(), "SP-001".into()],
            storeys: vec!["Level 1".into(), "Level 2".into()],
            ifc_types: vec!["IfcWall".into(), "IfcSlab".into()],
            severities: vec!["ERROR".into(), "WARNING".into(), "INFO".into()],
        }
    }

    fn finding() -> Finding {
        Finding {
            rule_id: "ID-001".into(),
            severity: Severity::Error,
            global_id: Some("0abcDEF123ghiJKL456mno".into()),
            step_id: Some(42),
            ifc_type: Some("IfcWall".into()),
            name: Some("Basic Wall 200".into()),
            storey: Some("Level 1".into()),
            message: "GlobalId is duplicated".into(),
            expected: Some("unique GlobalId".into()),
            actual: Some("used by 2 elements".into()),
            fix_hint: Some("Regenerate the GlobalId".into()),
            source: RuleSource::RulePack {
                pack: "generic".into(),
            },
        }
    }

    fn rule() -> RuleResult {
        RuleResult {
            rule_id: "ID-001".into(),
            category: "identity".into(),
            description: "GlobalId is unique".into(),
            severity: Severity::Error,
            status: RuleStatus::Fail,
            elements_checked: 10,
            failures: 2,
            skipped_reason: None,
            source: RuleSource::RulePack {
                pack: "generic".into(),
            },
        }
    }

    fn knowledge() -> Knowledge {
        Knowledge {
            chunks: vec![
                Chunk { source: "docs/x.md".into(), title: "Duplicate GlobalId".into(), text: "Rule ID-001 checks that every GlobalId is unique.".into(), keywords: vec!["id-001".into()] },
                Chunk { source: "docs/y.md".into(), title: "Required psets".into(), text: "requirements.required_psets lists pset, applies_to and properties. FireRating walls Pset_WallCommon.".into(), keywords: vec![] },
            ],
        }
    }

    fn run_mock(answers: &[&str], task: &Task) -> (Answer, MockBackend) {
        let mut b = MockBackend::new(answers.iter().copied());
        let a = run(&mut b, &knowledge(), task).unwrap();
        (a, b)
    }

    fn filter_task(q: &str) -> Task {
        Task::Filter {
            question: q.into(),
            context: ctx(),
        }
    }

    #[test]
    fn prompt_contains_retrieved_chunk_and_schema() {
        let (_, b) = run_mock(
            &[r#"{"answer":"It checks GlobalId uniqueness."}"#],
            &Task::Ask {
                question: "What does ID-001 check?".into(),
            },
        );
        let p = &b.prompts[0];
        assert!(p.contains("Duplicate GlobalId"), "{p}");
        assert!(p.contains(r#"{"answer": string}"#), "{p}");
        assert!(
            p.contains("you never decide") || p.contains("never decide whether anything passes"),
            "{p}"
        );
        assert!(p.starts_with("<|im_start|>system\n"));
        assert!(p.ends_with("<|im_start|>assistant\n{\"answer\":"), "{p}");
    }

    #[test]
    fn ask_valid_answer_has_sources() {
        let (a, _) = run_mock(
            &[r#"{"answer":"It checks GlobalId uniqueness."}"#],
            &Task::Ask {
                question: "What does ID-001 check?".into(),
            },
        );
        assert_eq!(
            a,
            Answer::Text {
                text: "It checks GlobalId uniqueness.".into(),
                sources: vec!["Duplicate GlobalId".into()],
                structured: true
            }
        );
    }

    #[test]
    fn ask_invalid_twice_falls_back_to_plain_text() {
        let (a, b) = run_mock(
            &["I think it checks ids", "still no json"],
            &Task::Ask {
                question: "What does ID-001 check?".into(),
            },
        );
        assert_eq!(
            a,
            Answer::Text {
                text: "still no json".into(),
                sources: vec!["Duplicate GlobalId".into()],
                structured: false
            }
        );
        assert_eq!(b.prompts.len(), 2);
        assert!(b.prompts[1].contains("Format error in your reply (no complete JSON object found)"));
        assert!(
            b.prompts[1].ends_with(
                "schema: {\"answer\": string}<|im_end|>\n<|im_start|>assistant\n{\"answer\":"
            ),
            "{}",
            b.prompts[1]
        );
        assert!(b.prompts[1].contains("I think it checks ids"));
    }

    #[test]
    fn primed_prefix_is_reattached() {
        // The model continues after `{"answer":` without repeating it.
        let (a, _) = run_mock(
            &[r#" "GlobalIds must be unique."}"#],
            &Task::Ask {
                question: "What does ID-001 check?".into(),
            },
        );
        assert!(
            matches!(a, Answer::Text { ref text, structured: true, .. } if text == "GlobalIds must be unique."),
            "{a:?}"
        );
        // Truncated twice: plain text without the JSON scaffolding.
        let (a, _) = run_mock(
            &[r#" "GlobalIds must"#, r#" "GlobalIds must be"#],
            &Task::Ask {
                question: "What does ID-001 check?".into(),
            },
        );
        assert!(
            matches!(a, Answer::Text { ref text, structured: false, .. } if text == "GlobalIds must be"),
            "{a:?}"
        );
    }

    #[test]
    fn filter_valid_with_canonicalisation() {
        let (a, b) = run_mock(
            &["```json\n{\"severity\": [\"errors\"], \"rule_id\": null, \"storey\": \"level 1\", \"ifc_type\": \"wall\", \"search\": \"\", \"explanation\": \"Errors on walls.\"}\n```"],
            &filter_task("show me the errors on Level 1 for walls"),
        );
        let Answer::Filter { filter } = a else {
            panic!("{a:?}")
        };
        assert_eq!(filter.severity, vec!["ERROR"]);
        assert_eq!(filter.storey.as_deref(), Some("Level 1"));
        assert_eq!(filter.ifc_type.as_deref(), Some("IfcWall"));
        assert_eq!(filter.rule_id, None);
        assert_eq!(filter.search, None);
        assert!(filter.dropped.is_empty());
        assert!(b.prompts[0].contains("rule_id: ID-001, SP-001"));
    }

    #[test]
    fn filter_invalid_json_then_valid_on_retry() {
        let (a, b) = run_mock(
            &[
                r#"{"severity": ["ERROR"], "storey": "Level 1""#,
                r#"{"severity": ["ERROR"], "storey": "Level 1"}"#,
            ],
            &filter_task("errors on level 1"),
        );
        let Answer::Filter { filter } = a else {
            panic!("{a:?}")
        };
        assert_eq!(filter.severity, vec!["ERROR"]);
        assert_eq!(filter.storey.as_deref(), Some("Level 1"));
        assert_eq!(b.prompts.len(), 2);
    }

    #[test]
    fn filter_hallucinated_values_are_dropped() {
        let (a, _) = run_mock(
            &[
                r#"{"severity": ["ERROR", "CRITICAL"], "rule_id": "XX-999", "storey": "Level 9", "ifc_type": "IfcSlab", "explanation": "x"}"#,
            ],
            &filter_task("critical slab problems"),
        );
        let Answer::Filter { filter } = a else {
            panic!("{a:?}")
        };
        assert_eq!(filter.rule_id, None);
        assert_eq!(filter.storey, None);
        assert_eq!(filter.ifc_type.as_deref(), Some("IfcSlab"));
        assert_eq!(filter.severity, vec!["ERROR"]);
        assert_eq!(
            filter.dropped,
            vec!["severity: CRITICAL", "rule_id: XX-999", "storey: Level 9"]
        );
    }

    #[test]
    fn explain_valid_and_prompt_has_facts() {
        let task = Task::Explain {
            finding: Box::new(finding()),
            rule: Some(rule()),
        };
        let (a, b) = run_mock(
            &[
                r#"{"text": "Basic Wall 200 (0abcDEF123ghiJKL456mno) shares its GlobalId. Regenerate it."}"#,
            ],
            &task,
        );
        assert_eq!(
            a,
            Answer::Explanation {
                text: "Basic Wall 200 (0abcDEF123ghiJKL456mno) shares its GlobalId. Regenerate it."
                    .into()
            }
        );
        let p = &b.prompts[0];
        assert!(
            p.contains(
                "IfcWall \"Basic Wall 200\" (GlobalId 0abcDEF123ghiJKL456mno), storey Level 1"
            ),
            "{p}"
        );
        assert!(p.contains("Rule: ID-001 [identity] GlobalId is unique. Status Fail, 2 failures out of 10 elements."), "{p}");
        assert!(p.contains("Duplicate GlobalId"));
    }

    #[test]
    fn explain_invalid_then_valid_on_retry() {
        let task = Task::Explain {
            finding: Box::new(finding()),
            rule: None,
        };
        let (a, _) = run_mock(
            &[
                "The wall has a duplicated id.",
                "{\"text\": \"Line one.\nLine two.\"}",
            ],
            &task,
        );
        assert_eq!(
            a,
            Answer::Explanation {
                text: "Line one.\nLine two.".into()
            }
        );
    }

    #[test]
    fn config_help_valid_snippet() {
        let task = Task::ConfigHelp {
            question: "require FireRating on all walls".into(),
            current_yaml: Some("project:\n  name: Demo\n".into()),
        };
        let yaml = "requirements:\n  required_psets:\n    - pset: Pset_WallCommon\n      applies_to: [IfcWall]\n      properties: [FireRating]\n";
        let out =
            serde_json::json!({"yaml": yaml, "explanation": "Walls need FireRating."}).to_string();
        let (a, b) = run_mock(&[out.as_str()], &task);
        assert_eq!(
            a,
            Answer::ConfigSnippet {
                yaml: yaml.into(),
                valid: true,
                validation_error: None,
                explanation: "Walls need FireRating.".into()
            }
        );
        assert!(b.prompts[0].contains("Current project.yaml:\nproject:\n  name: Demo"));
        assert!(b.prompts[0].contains(r#"{"yaml": string, "explanation": string}"#));
    }

    #[test]
    fn config_help_invalid_yaml_is_flagged_after_retry() {
        let task = Task::ConfigHelp {
            question: "require FireRating on all walls".into(),
            current_yaml: None,
        };
        let bad = serde_json::json!({"yaml": "requirements:\n  required_psets:\n    - name: Pset_WallCommon\n", "explanation": "x"}).to_string();
        let (a, b) = run_mock(&[bad.as_str(), bad.as_str()], &task);
        let Answer::ConfigSnippet {
            valid,
            validation_error,
            yaml,
            ..
        } = a
        else {
            panic!("{a:?}")
        };
        assert!(!valid);
        assert!(validation_error.unwrap().contains("unknown field"));
        assert!(yaml.contains("Pset_WallCommon"));
        assert_eq!(b.prompts.len(), 2);
        assert!(b.prompts[1].contains("not a valid VeriModel configuration"));
    }

    #[test]
    fn config_help_invalid_then_valid_on_retry() {
        let task = Task::ConfigHelp {
            question: "door names D-001".into(),
            current_yaml: None,
        };
        let bad =
            serde_json::json!({"yaml": "naming:\n  patterns:\n    IfcDoor: \"^D-[0-9{3}$\"\n"})
                .to_string();
        let good =
            serde_json::json!({"yaml": "naming:\n  patterns:\n    IfcDoor: \"^D-[0-9]{3}$\"\n"})
                .to_string();
        let (a, _) = run_mock(&[bad.as_str(), good.as_str()], &task);
        assert!(
            matches!(a, Answer::ConfigSnippet { valid: true, .. }),
            "{a:?}"
        );
    }

    #[test]
    fn draft_rules_fragment_is_wrapped_and_validated() {
        let task = Task::DraftRules {
            requirements_text:
                "All walls shall carry Pset_WallCommon.FireRating. Walls are named W-###.".into(),
        };
        // Literal newlines inside the JSON string: repaired by the parser.
        let out = "{\"yaml\": \"required_psets:\n  - pset: Pset_WallCommon\n    applies_to: [IfcWall]\n    properties: [FireRating]\npatterns:\n  IfcWall: '^W-[0-9]{3}$'\n\", \"explanation\": \"From the two sentences.\"}";
        let (a, b) = run_mock(&[out], &task);
        let Answer::ConfigSnippet {
            yaml,
            valid,
            validation_error,
            ..
        } = a
        else {
            panic!("{a:?}")
        };
        assert!(valid, "{validation_error:?}\n{yaml}");
        assert!(
            yaml.starts_with("requirements:\n  required_psets:"),
            "{yaml}"
        );
        assert!(yaml.contains("naming:\n  patterns:"), "{yaml}");
        assert!(b.prompts[0].contains("Requirements text:\nAll walls shall carry"));
    }

    #[test]
    fn draft_rules_invalid_twice_flagged_not_applied() {
        let task = Task::DraftRules {
            requirements_text: "Walls need a fire rating.".into(),
        };
        let bad = serde_json::json!({"yaml": "required_psets:\n  - pset: Pset_WallCommon\n    severity: fatal\n"}).to_string();
        let (a, _) = run_mock(&[bad.as_str(), bad.as_str()], &task);
        let Answer::ConfigSnippet {
            valid,
            validation_error,
            ..
        } = a
        else {
            panic!("{a:?}")
        };
        assert!(!valid);
        assert!(validation_error
            .unwrap()
            .contains("severity 'fatal' is invalid"));
    }

    #[test]
    fn non_json_config_answer_falls_back_to_text() {
        let task = Task::ConfigHelp {
            question: "x".into(),
            current_yaml: None,
        };
        let (a, _) = run_mock(&["no idea", "still no idea"], &task);
        assert!(
            matches!(
                a,
                Answer::Text {
                    structured: false,
                    ..
                }
            ),
            "{a:?}"
        );
    }

    #[test]
    fn backend_errors_propagate() {
        let mut b = MockBackend::new(Vec::<String>::new());
        assert!(run(
            &mut b,
            &knowledge(),
            &Task::Ask {
                question: "x".into()
            }
        )
        .is_err());
    }

    #[test]
    fn salvage_keeps_complete_leading_pairs() {
        let m = salvage_leading_pairs(
            r#"{"answer": "Every element must have a unique GlobalId.", "id": 001}"#,
        )
        .unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m["answer"], "Every element must have a unique GlobalId.");
        assert!(salvage_leading_pairs(r#"{"answer": "trunc"#).is_none());
        let (a, b) = run_mock(
            &[r#" "Unique GlobalIds.", "id": 001}"#],
            &Task::Ask {
                question: "What does ID-001 check?".into(),
            },
        );
        assert!(
            matches!(a, Answer::Text { ref text, structured: true, .. } if text == "Unique GlobalIds."),
            "{a:?}"
        );
        assert_eq!(b.prompts.len(), 1);
        // Filters are never salvaged: a lost field would widen the filter.
        let (_, b) = run_mock(
            &[
                r#"{"severity": ["ERROR"], "storey": 01}"#,
                r#"{"severity": ["ERROR"]}"#,
            ],
            &filter_task("errors"),
        );
        assert_eq!(b.prompts.len(), 2);
    }

    #[test]
    fn json_extraction() {
        assert_eq!(
            extract_json_object("text {\"a\": \"}{\", \"b\": {\"c\": 1}} tail {}"),
            Some("{\"a\": \"}{\", \"b\": {\"c\": 1}}")
        );
        assert_eq!(extract_json_object("{\"a\": 1"), None);
        assert!(parse_json_object("[1,2]").is_err());
    }

    #[test]
    fn templates_parse() {
        for t in [
            PROMPT_FILTER,
            PROMPT_EXPLAIN,
            PROMPT_CONFIG_HELP,
            PROMPT_DRAFT_RULES,
            PROMPT_ASK,
        ] {
            let (system, turns) = parse_template(t);
            assert!(system.contains("JSON schema:"), "{system}");
            assert!(turns.len() % 2 == 0);
            for pair in turns.chunks(2) {
                assert_eq!(pair[0].role, "user");
                assert_eq!(pair[1].role, "assistant");
                assert!(
                    parse_json_object(&pair[1].content).is_ok(),
                    "{}",
                    pair[1].content
                );
            }
        }
        // Few-shot config examples must themselves be valid configurations.
        for t in [PROMPT_CONFIG_HELP, PROMPT_DRAFT_RULES] {
            for turn in parse_template(t).1.iter().filter(|m| m.role == "assistant") {
                let a = parse_snippet(&turn.content).unwrap();
                assert!(
                    matches!(a, Answer::ConfigSnippet { valid: true, .. }),
                    "{a:?}"
                );
            }
        }
    }
}
