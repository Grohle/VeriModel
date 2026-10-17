//! Documentation → knowledge YAML.
//!
//! Turns project documentation (BIM execution plans, employer's information
//! requirements, naming conventions, ...) into [`Knowledge`] chunks the
//! assistant can retrieve. Pure text extraction, no model involved.
//!
//! Contract:
//! * `convert_paths(paths)` accepts files and directories (recursed) with
//!   extensions .md .markdown .txt .yaml .yml .json .docx .pdf; others are
//!   skipped and listed in `ConvertReport.skipped`.
//! * .md/.txt: `knowledge::chunk_markdown`. .yaml/.yml: if it already is a
//!   Knowledge file, merge; else chunk the YAML text by top-level keys.
//!   .json: pretty-print and chunk. .docx: unzip `word/document.xml`, take
//!   `w:p` paragraphs (quick-xml), headings from `w:pStyle` Heading1..3.
//!   .pdf: `pdf_extract::extract_text`, paragraphs split on blank lines.
//! * Every chunk gets `source` = relative path, `keywords` = top 8 distinct
//!   capitalised or underscored tokens (identifiers like Pset_WallCommon).
//! * Deterministic order (sorted paths).
//!
//! Notes: "capitalised or underscored tokens" means identifier-like tokens:
//! containing `_`, an inner capital (`IfcWall`, `BIM`) or a capital plus a
//! digit (`ID-001`); ordinary capitalised words ("The") are not keywords.
//! Hidden files and directories (`.git`, ...) are ignored when recursing.
//! A file that cannot be read or parsed (e.g. a broken PDF) is listed in
//! `skipped` with the reason; it never fails the whole conversion.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AssistantResult;
use crate::knowledge::{chunk_markdown, split_long, Chunk, Knowledge};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConvertReport {
    pub knowledge: Knowledge,
    pub files_read: Vec<String>,
    pub skipped: Vec<String>,
}

pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "yaml", "yml", "json", "docx", "pdf",
];

/// Maximum characters per chunk (same as markdown chunking).
const MAX_CHUNK_CHARS: usize = 1800;
/// Number of keywords per chunk.
const KEYWORDS: usize = 8;

/// Convert files/directories into knowledge.
pub fn convert_paths(paths: &[PathBuf]) -> AssistantResult<ConvertReport> {
    let mut report = ConvertReport::default();
    // (absolute-ish path used for ordering, source label)
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    for p in paths {
        if p.is_dir() {
            collect_dir(p, p, &mut files);
        } else if p.is_file() {
            let label = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string());
            files.push((p.clone(), label));
        } else {
            report.skipped.push(format!("{}: not found", p.display()));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.dedup_by(|a, b| a.0 == b.0);

    for (path, source) in files {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
            report.skipped.push(format!(
                "{source}: unsupported file type{}",
                if ext.is_empty() {
                    String::new()
                } else {
                    format!(" .{ext}")
                }
            ));
            continue;
        }
        match convert_file(&path, &source, &ext) {
            Ok(mut chunks) => {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| source.clone());
                for c in &mut chunks {
                    if c.title.trim().is_empty() {
                        c.title = stem.clone();
                    }
                    if c.keywords.is_empty() {
                        c.keywords = keywords(&format!("{}\n{}", c.title, c.text), KEYWORDS);
                    }
                }
                report.knowledge.chunks.extend(chunks);
                report.files_read.push(source);
            }
            Err(reason) => report.skipped.push(format!("{source}: {reason}")),
        }
    }
    Ok(report)
}

/// Recurse into `dir`, labelling files relative to `root` with `/` separators.
fn collect_dir(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.file_name()
            .map(|n| n.to_string_lossy().starts_with('.'))
            .unwrap_or(false)
        {
            continue;
        }
        if p.is_dir() {
            collect_dir(root, &p, out);
        } else if p.is_file() {
            let rel = p.strip_prefix(root).unwrap_or(&p);
            let label = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((p, label));
        }
    }
}

fn convert_file(path: &Path, source: &str, ext: &str) -> Result<Vec<Chunk>, String> {
    match ext {
        "md" | "markdown" | "txt" => Ok(chunk_markdown(source, &read_text(path)?)),
        "yaml" | "yml" => convert_yaml(source, &read_text(path)?),
        "json" => convert_json(source, &read_text(path)?),
        "docx" => Ok(chunk_markdown(source, &docx_to_markdown(path)?)),
        "pdf" => Ok(chunk_markdown(source, &pdf_to_text(path)?)),
        other => Err(format!("unsupported file type .{other}")),
    }
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// A Knowledge file (top-level `chunks:`) is merged; any other YAML is chunked by top-level key.
fn convert_yaml(source: &str, text: &str) -> Result<Vec<Chunk>, String> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(text).map_err(|e| format!("invalid YAML: {e}"))?;
    if let serde_yaml::Value::Mapping(m) = &value {
        if m.contains_key("chunks") {
            let k =
                Knowledge::from_yaml(text).map_err(|e| format!("invalid knowledge file: {e}"))?;
            return Ok(k
                .chunks
                .into_iter()
                .map(|mut c| {
                    if c.source.trim().is_empty() {
                        c.source = source.to_string();
                    }
                    c
                })
                .collect());
        }
        let mut out = Vec::new();
        for (k, v) in m {
            let key = match k {
                serde_yaml::Value::String(s) => s.clone(),
                other => serde_yaml::to_string(other)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            };
            let mut one = serde_yaml::Mapping::new();
            one.insert(k.clone(), v.clone());
            let body = serde_yaml::to_string(&one).unwrap_or_default();
            out.extend(pieces(source, &key, &body));
        }
        return Ok(out);
    }
    Ok(pieces(source, "", text))
}

/// JSON is pretty-printed; an object is chunked by top-level key.
fn convert_json(source: &str, text: &str) -> Result<Vec<Chunk>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    if let serde_json::Value::Object(m) = &value {
        let mut out = Vec::new();
        for (k, v) in m {
            let mut one = serde_json::Map::new();
            one.insert(k.clone(), v.clone());
            let body =
                serde_json::to_string_pretty(&serde_json::Value::Object(one)).unwrap_or_default();
            out.extend(pieces(source, k, &body));
        }
        return Ok(out);
    }
    Ok(pieces(
        source,
        "",
        &serde_json::to_string_pretty(&value).unwrap_or_default(),
    ))
}

fn pieces(source: &str, title: &str, text: &str) -> Vec<Chunk> {
    let t = text.trim();
    if t.is_empty() {
        return vec![];
    }
    split_long(t, MAX_CHUNK_CHARS)
        .into_iter()
        .map(|text| Chunk {
            source: source.to_string(),
            title: title.to_string(),
            text,
            keywords: vec![],
        })
        .collect()
}

/// Extract `word/document.xml` paragraphs as markdown (Title/Heading1..3 → #..####).
fn docx_to_markdown(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot read: {e}"))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("not a valid .docx (zip): {e}"))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|e| format!("not a valid .docx (word/document.xml): {e}"))?
        .read_to_string(&mut xml)
        .map_err(|e| format!("cannot read word/document.xml: {e}"))?;
    docx_xml_to_markdown(&xml)
}

fn docx_xml_to_markdown(xml: &str) -> Result<String, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut paragraphs: Vec<String> = Vec::new();
    let mut para = String::new();
    let mut style: Option<String> = None;
    let mut in_para = false;
    let mut in_text = false;
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("invalid word/document.xml: {e}"))?
        {
            Event::Start(e) => match e.name().as_ref() {
                b"w:p" => {
                    in_para = true;
                    para.clear();
                    style = None;
                }
                b"w:t" => in_text = true,
                _ => {}
            },
            Event::Empty(e) => match e.name().as_ref() {
                b"w:pStyle" if in_para => {
                    style = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == b"w:val")
                        .and_then(|a| a.unescape_value().ok())
                        .map(|v| v.into_owned());
                }
                b"w:tab" if in_para => para.push('\t'),
                b"w:br" | b"w:cr" if in_para => para.push('\n'),
                _ => {}
            },
            Event::Text(t) if in_text => {
                para.push_str(
                    &t.unescape()
                        .map_err(|e| format!("invalid text in word/document.xml: {e}"))?,
                );
            }
            Event::End(e) => match e.name().as_ref() {
                b"w:t" => in_text = false,
                b"w:p" => {
                    in_para = false;
                    let text = para.trim();
                    if !text.is_empty() {
                        let prefix = match style
                            .as_deref()
                            .map(|s| s.to_lowercase().replace([' ', '-', '_'], ""))
                            .as_deref()
                        {
                            Some("title") => "# ",
                            Some("heading1") => "## ",
                            Some("heading2") => "### ",
                            Some("heading3") => "#### ",
                            _ => "",
                        };
                        let text = if prefix.is_empty() {
                            text.to_string()
                        } else {
                            text.replace(['\n', '\t'], " ")
                        };
                        paragraphs.push(format!("{prefix}{text}"));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(paragraphs.join("\n\n"))
}

/// Extract PDF text; paragraphs are separated by blank lines.
fn pdf_to_text(path: &Path) -> Result<String, String> {
    let p = path.to_path_buf();
    let text = std::panic::catch_unwind(move || pdf_extract::extract_text(&p))
        .map_err(|_| "the PDF could not be parsed".to_string())?
        .map_err(|e| format!("no text could be extracted from the PDF: {e}"))?;
    let paragraphs: Vec<String> = text
        .replace('\r', "")
        .split("\n\n")
        .map(|p| {
            p.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|p| !p.is_empty())
        .collect();
    if paragraphs.is_empty() {
        return Err("the PDF contains no extractable text (scanned image?)".into());
    }
    Ok(paragraphs.join("\n\n"))
}

/// Identifier-like token: contains `_`, an inner capital, or a capital and a digit.
fn is_identifier(t: &str) -> bool {
    if t.chars().count() < 2 || !t.chars().any(|c| c.is_alphabetic()) {
        return false;
    }
    let has_inner_upper = t.chars().skip(1).any(|c| c.is_uppercase());
    let starts_upper = t.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
    t.contains('_') || has_inner_upper || (starts_upper && t.chars().any(|c| c.is_ascii_digit()))
}

/// Top `n` distinct identifier-like tokens by frequency (ties: first occurrence).
pub fn keywords(text: &str, n: usize) -> Vec<String> {
    let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
    for (i, tok) in text
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .map(|t| t.trim_matches(|c| c == '-' || c == '_'))
        .filter(|t| is_identifier(t))
        .enumerate()
    {
        let e = counts.entry(tok.to_string()).or_insert((0, i));
        e.0 += 1;
    }
    let mut v: Vec<(String, (usize, usize))> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.1 .1.cmp(&b.1 .1)));
    v.into_iter().take(n).map(|(t, _)| t).collect()
}

/// Convert and write `knowledge.yaml` next to the outputs.
pub fn convert_to_file(paths: &[PathBuf], out: &Path) -> AssistantResult<ConvertReport> {
    let r = convert_paths(paths)?;
    std::fs::write(out, r.knowledge.to_yaml())?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_docx(path: &Path) {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Property requirements</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">All walls shall carry </w:t></w:r><w:r><w:t>Pset_WallCommon.FireRating &amp; IsExternal.</w:t></w:r></w:p>
<w:p><w:r><w:t>Doors use Pset_DoorCommon; Pset_WallCommon applies to IfcWall only.</w:t></w:r></w:p>
<w:sectPr/></w:body></w:document>"#;
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        z.start_file("[Content_Types].xml", opts).unwrap();
        z.write_all(b"<Types/>").unwrap();
        z.start_file("word/document.xml", opts).unwrap();
        z.write_all(xml.as_bytes()).unwrap();
        z.finish().unwrap();
    }

    /// A minimal uncompressed one-page PDF with two text paragraphs.
    fn write_pdf(path: &Path) {
        let content = "BT /F1 12 Tf 72 720 Td (BIM Execution Plan) Tj ET\nBT /F1 12 Tf 72 600 Td (Walls require Pset_WallCommon) Tj ET\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{}\nendobj\n", i + 1, o));
        }
        let xref = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        for o in offsets {
            pdf.push_str(&format!("{o:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref
        ));
        std::fs::write(path, pdf).unwrap();
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(
            d.join("a_guide.md"),
            "# Guide\nintro text\n## Naming\nWalls are named W-001 using IfcWall names.\n",
        )
        .unwrap();
        std::fs::write(
            d.join("b_notes.txt"),
            "Plain notes about the BIM model.\n\nSecond paragraph mentions LOD_300.",
        )
        .unwrap();
        std::fs::write(
            d.join("c_data.json"),
            r#"{"project": {"name": "Demo"}, "codes": ["Ss_25_10", "Ss_25_20"]}"#,
        )
        .unwrap();
        std::fs::write(
            d.join("d_knowledge.yaml"),
            "chunks:\n  - source: ''\n    title: Existing chunk\n    text: Some imported text about Pset_SlabCommon.\n    keywords: [slab]\n",
        )
        .unwrap();
        std::fs::write(
            d.join("e_config.yml"),
            "naming:\n  patterns:\n    IfcWall: '^W-'\nqto:\n  currency: EUR\n",
        )
        .unwrap();
        write_docx(&d.join("f_eir.docx"));
        std::fs::write(d.join("g_image.xyz"), "binary").unwrap();
        std::fs::create_dir(d.join("sub")).unwrap();
        std::fs::write(
            d.join("sub").join("h_more.md"),
            "## Deep\nNested IfcSlab note.\n",
        )
        .unwrap();
        std::fs::create_dir(d.join(".git")).unwrap();
        std::fs::write(d.join(".git").join("x.md"), "ignored").unwrap();
        dir
    }

    #[test]
    fn converts_mixed_directory() {
        let dir = fixture();
        let r = convert_paths(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(
            r.files_read,
            vec![
                "a_guide.md",
                "b_notes.txt",
                "c_data.json",
                "d_knowledge.yaml",
                "e_config.yml",
                "f_eir.docx",
                "sub/h_more.md"
            ]
        );
        assert_eq!(r.skipped, vec!["g_image.xyz: unsupported file type .xyz"]);

        let by_source = |s: &str| {
            r.knowledge
                .chunks
                .iter()
                .filter(|c| c.source == s)
                .collect::<Vec<_>>()
        };
        let md = by_source("a_guide.md");
        assert_eq!(md.len(), 2);
        assert_eq!(md[1].title, "Guide › Naming");
        assert!(
            md[1].keywords.contains(&"W-001".to_string()),
            "{:?}",
            md[1].keywords
        );
        assert!(md[1].keywords.contains(&"IfcWall".to_string()));
        assert!(!md[1].keywords.contains(&"Walls".to_string()));

        let txt = by_source("b_notes.txt");
        assert_eq!(txt.len(), 1);
        assert_eq!(txt[0].title, "b_notes");
        assert_eq!(txt[0].keywords, vec!["b_notes", "BIM", "LOD_300"]);

        let json = by_source("c_data.json");
        assert_eq!(
            json.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            vec!["project", "codes"]
        );
        assert!(json[1].keywords.contains(&"Ss_25_10".to_string()));

        let k = by_source("d_knowledge.yaml");
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].title, "Existing chunk");
        assert_eq!(k[0].keywords, vec!["slab"]);

        let y = by_source("e_config.yml");
        assert_eq!(
            y.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            vec!["naming", "qto"]
        );
        assert!(y[0].text.contains("IfcWall"));

        let docx = by_source("f_eir.docx");
        assert_eq!(docx.len(), 1);
        assert_eq!(docx[0].title, "Property requirements");
        assert!(
            docx[0]
                .text
                .contains("All walls shall carry Pset_WallCommon.FireRating & IsExternal."),
            "{}",
            docx[0].text
        );
        assert!(docx[0].text.contains("\n\nDoors use Pset_DoorCommon"));
        assert_eq!(docx[0].keywords[0], "Pset_WallCommon");

        assert_eq!(by_source("sub/h_more.md")[0].title, "Deep");
        assert_eq!(r.knowledge.chunks.len(), 2 + 1 + 2 + 1 + 2 + 1 + 1);

        // Deterministic: same output twice; a file reached twice is read once.
        assert_eq!(convert_paths(&[dir.path().to_path_buf()]).unwrap(), r);
        let again = convert_paths(&[dir.path().join("sub"), dir.path().to_path_buf()]).unwrap();
        assert_eq!(again.knowledge.chunks.len(), r.knowledge.chunks.len());
    }

    #[test]
    fn missing_and_broken_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let bad_json = dir.path().join("bad.json");
        let bad_docx = dir.path().join("bad.docx");
        let bad_pdf = dir.path().join("bad.pdf");
        std::fs::write(&bad_json, "{ nope").unwrap();
        std::fs::write(&bad_docx, "not a zip").unwrap();
        std::fs::write(&bad_pdf, "%PDF-1.4 garbage").unwrap();
        let r =
            convert_paths(&[bad_json, bad_docx, bad_pdf, dir.path().join("missing.md")]).unwrap();
        assert!(r.files_read.is_empty());
        assert_eq!(r.skipped.len(), 4, "{:?}", r.skipped);
        assert!(r.skipped[0].ends_with("missing.md: not found"));
        assert!(r.skipped[1].starts_with("bad.docx: not a valid .docx"));
        assert!(r.skipped[2].starts_with("bad.json: invalid JSON"));
        assert!(r.skipped[3].starts_with("bad.pdf: "));
    }

    #[test]
    fn converts_minimal_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bep.pdf");
        write_pdf(&p);
        let r = convert_paths(std::slice::from_ref(&p)).unwrap();
        assert_eq!(r.files_read, vec!["bep.pdf"], "{:?}", r.skipped);
        let text: String = r
            .knowledge
            .chunks
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("BIM Execution Plan"), "{text}");
        assert!(text.contains("Pset_WallCommon"), "{text}");
        assert!(r.knowledge.chunks[0]
            .keywords
            .contains(&"Pset_WallCommon".to_string()));
    }

    #[test]
    fn convert_to_file_writes_loadable_knowledge() {
        let dir = fixture();
        let out = dir.path().join("out.yaml");
        let r = convert_to_file(&[dir.path().join("a_guide.md")], &out).unwrap();
        let k = Knowledge::from_yaml(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(k, r.knowledge);
        assert!(!k.search("naming walls", 1).is_empty());
    }

    #[test]
    fn keyword_rules() {
        assert_eq!(
            keywords(
                "The Wall uses IfcWall and IfcWall, Pset_WallCommon, ID-001, plain words, X",
                8
            ),
            vec!["IfcWall", "Pset_WallCommon", "ID-001"]
        );
    }
}
