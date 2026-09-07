//! Document parsing: PDF (`pdf-extract`), DOCX (`zip` + `quick-xml`), TXT/MD.
//! Everything is pure Rust — no C toolchain, no external binaries.

use std::io::{Cursor, Read};
use std::path::Path;

use bluey_core::error::BlueyError;
use bluey_core::types::documents::DocumentFormat;
use quick_xml::events::Event;

/// Hard input limit — larger blobs are rejected with `storage.document_too_large`.
pub const MAX_DOCUMENT_BYTES: usize = 20 * 1024 * 1024;

/// Result of parsing a document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedDocument {
    /// Normalized plain text (Unix line endings, control chars stripped,
    /// runs of 3+ newlines collapsed, trimmed).
    pub text: String,
    /// Best-effort title (DOCX core properties, first Markdown `#` heading).
    pub title: Option<String>,
    /// Small format-specific metadata (byte size, paragraph counts, …).
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

/// Detect a [`DocumentFormat`] from a file extension.
pub fn detect_format(path: &Path) -> Option<DocumentFormat> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(DocumentFormat::from_extension)
}

/// Parse `bytes` as `format` into normalized plain text.
///
/// Errors: `storage.document_too_large` (> [`MAX_DOCUMENT_BYTES`]),
/// `storage.parse` (malformed PDF/DOCX), `storage.document_empty` (no text).
pub fn parse_document(bytes: &[u8], format: DocumentFormat) -> Result<ParsedDocument, BlueyError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(BlueyError::storage(
            "document_too_large",
            format!(
                "document is {} bytes; the limit is {} bytes",
                bytes.len(),
                MAX_DOCUMENT_BYTES
            ),
        ));
    }
    let mut parsed = match format {
        DocumentFormat::Pdf => parse_pdf(bytes)?,
        DocumentFormat::Docx => parse_docx(bytes)?,
        DocumentFormat::Txt | DocumentFormat::Text => parse_plain(bytes, false),
        DocumentFormat::Md => parse_plain(bytes, true),
    };
    parsed.text = normalize_text(&parsed.text);
    parsed
        .metadata
        .insert("sourceBytes".into(), serde_json::json!(bytes.len()));
    parsed
        .metadata
        .insert("format".into(), serde_json::json!(format.as_str()));
    if parsed.text.is_empty() {
        return Err(BlueyError::storage(
            "document_empty",
            "the document contains no extractable text",
        ));
    }
    Ok(parsed)
}

fn parse_pdf(bytes: &[u8]) -> Result<ParsedDocument, BlueyError> {
    let text = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| BlueyError::storage("parse", format!("PDF text extraction failed: {e}")))?;
    Ok(ParsedDocument {
        text,
        title: None,
        metadata: serde_json::Map::new(),
    })
}

fn parse_plain(bytes: &[u8], markdown: bool) -> ParsedDocument {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let title = if markdown {
        text.lines().find_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("# ")
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        })
    } else {
        None
    };
    ParsedDocument {
        text,
        title,
        metadata: serde_json::Map::new(),
    }
}

fn parse_docx(bytes: &[u8]) -> Result<ParsedDocument, BlueyError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| {
        BlueyError::storage("parse", format!("DOCX is not a valid zip archive: {e}"))
    })?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|e| BlueyError::storage("parse", format!("DOCX has no word/document.xml: {e}")))?
        .read_to_string(&mut xml)
        .map_err(|e| BlueyError::storage("parse", format!("cannot read word/document.xml: {e}")))?;
    let (text, paragraphs) = extract_docx_text(&xml)?;

    // Best-effort title from docProps/core.xml.
    let mut title = None;
    if let Ok(mut core) = archive.by_name("docProps/core.xml") {
        let mut core_xml = String::new();
        if core.read_to_string(&mut core_xml).is_ok() {
            title = extract_docx_title(&core_xml);
        }
    }

    let mut metadata = serde_json::Map::new();
    metadata.insert("paragraphs".into(), serde_json::json!(paragraphs));
    Ok(ParsedDocument {
        text,
        title,
        metadata,
    })
}

/// Walk `word/document.xml`: text from `<w:t>`, paragraph breaks on `</w:p>`,
/// tabs on `<w:tab/>`, line breaks on `<w:br/>`; deleted text (`<w:delText>`,
/// inside `<w:del>`) and field instructions (`<w:instrText>`) are skipped.
fn extract_docx_text(xml: &str) -> Result<(String, u64), BlueyError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut paragraphs = 0u64;
    let mut in_text = false;
    let mut skip_depth = 0u32; // inside <w:del> / <w:delText> / <w:instrText>
    loop {
        match reader
            .read_event()
            .map_err(|e| BlueyError::storage("parse", format!("invalid DOCX XML: {e}")))?
        {
            Event::Start(e) => match e.local_name().as_ref() {
                b"t" => in_text = true,
                b"del" | b"delText" | b"instrText" => skip_depth += 1,
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"del" | b"delText" | b"instrText" => skip_depth = skip_depth.saturating_sub(1),
                b"p" => {
                    paragraphs += 1;
                    out.push('\n');
                }
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"tab" if skip_depth == 0 => out.push('\t'),
                b"br" | b"cr" if skip_depth == 0 => out.push('\n'),
                _ => {}
            },
            Event::Text(t) => {
                if in_text && skip_depth == 0 {
                    let text = t.unescape().map_err(|e| {
                        BlueyError::storage("parse", format!("invalid DOCX text: {e}"))
                    })?;
                    out.push_str(&text);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((out, paragraphs))
}

/// Pull `<dc:title>` text out of `docProps/core.xml`.
fn extract_docx_title(xml: &str) -> Option<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut in_title = false;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) if e.local_name().as_ref() == b"title" => in_title = true,
            Event::End(e) if e.local_name().as_ref() == b"title" => in_title = false,
            Event::Text(t) if in_title => {
                let title = t.unescape().ok()?.trim().to_string();
                if !title.is_empty() {
                    return Some(title);
                }
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// Normalize extracted text: Unix line endings, control characters stripped
/// (except `\n` / `\t`), trailing per-line whitespace removed, runs of 3+
/// newlines collapsed to a single blank line, outer whitespace trimmed.
pub(crate) fn normalize_text(input: &str) -> String {
    let unified = input.replace("\r\n", "\n").replace('\r', "\n");
    let cleaned: String = unified
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let mut lines: Vec<&str> = cleaned.lines().map(str::trim_end).collect();
    // Drop leading/trailing empty lines.
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let mut out = String::with_capacity(cleaned.len());
    let mut blank_run = 0u32;
    for line in lines {
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue; // collapse to a single blank line
            }
        } else {
            blank_run = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::io::Write;

    pub(crate) fn build_docx(document_xml: &str, core_xml: Option<&str>) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("word/document.xml", options).unwrap();
        writer.write_all(document_xml.as_bytes()).unwrap();
        if let Some(core) = core_xml {
            writer.start_file("docProps/core.xml", options).unwrap();
            writer.write_all(core.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// A minimal one-page PDF with a Helvetica text object, with a correct xref
    /// table computed at build time.
    pub(crate) fn build_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
                .to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
        }
        let xref_offset = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        for off in &offsets {
            pdf.push_str(&format!("{off:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            objects.len() + 1
        ));
        pdf.into_bytes()
    }

    #[test]
    fn parses_plain_text_and_markdown_title() {
        let parsed = parse_document(
            b"  Hello\r\nWorld\r\n\r\n\r\n\r\nEnd  ",
            DocumentFormat::Txt,
        )
        .unwrap();
        assert_eq!(parsed.text, "Hello\nWorld\n\nEnd");
        assert!(parsed.title.is_none());

        let md = "# My Resume\n\nSome text";
        let parsed = parse_document(md.as_bytes(), DocumentFormat::Md).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("My Resume"));
    }

    #[test]
    fn strips_control_chars_and_rejects_empty() {
        let parsed = parse_document(b"a\x00b\x07c", DocumentFormat::Text).unwrap();
        assert_eq!(parsed.text, "abc");
        let err = parse_document(b"\x00\x01", DocumentFormat::Text).unwrap_err();
        assert_eq!(err.code, "storage.document_empty");
    }

    #[test]
    fn rejects_oversized_documents() {
        let big = vec![b'a'; MAX_DOCUMENT_BYTES + 1];
        let err = parse_document(&big, DocumentFormat::Txt).unwrap_err();
        assert_eq!(err.code, "storage.document_too_large");
    }

    #[test]
    fn detects_format_from_extension() {
        assert_eq!(
            detect_format(Path::new("/x/resume.PDF")),
            Some(DocumentFormat::Pdf)
        );
        assert_eq!(
            detect_format(Path::new("notes.markdown")),
            Some(DocumentFormat::Md)
        );
        assert_eq!(
            detect_format(Path::new("doc.docx")),
            Some(DocumentFormat::Docx)
        );
        assert_eq!(detect_format(Path::new("no_extension")), None);
        assert_eq!(detect_format(Path::new("image.png")), None);
    }

    #[test]
    fn parses_docx_with_tabs_breaks_deleted_text_and_title() {
        let document = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:tab/><w:t>World</w:t></w:r></w:p>
    <w:p><w:del><w:r><w:delText>REMOVED</w:delText></w:r></w:del><w:r><w:t>Kept</w:t></w:r></w:p>
    <w:p><w:r><w:t>Line1</w:t><w:br/><w:t>Line2</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let core = r#"<?xml version="1.0"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
  xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Quarterly Notes</dc:title></cp:coreProperties>"#;
        let bytes = build_docx(document, Some(core));
        let parsed = parse_document(&bytes, DocumentFormat::Docx).unwrap();
        assert_eq!(parsed.text, "Hello\tWorld\nKept\nLine1\nLine2");
        assert!(!parsed.text.contains("REMOVED"));
        assert_eq!(parsed.title.as_deref(), Some("Quarterly Notes"));
        assert_eq!(
            parsed.metadata.get("paragraphs"),
            Some(&serde_json::json!(3))
        );
    }

    #[test]
    fn invalid_docx_is_a_parse_error() {
        let err = parse_document(b"definitely not a zip", DocumentFormat::Docx).unwrap_err();
        assert_eq!(err.code, "storage.parse");

        // Valid zip but missing word/document.xml.
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("other.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"x").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let err = parse_document(&bytes, DocumentFormat::Docx).unwrap_err();
        assert_eq!(err.code, "storage.parse");
    }

    #[test]
    fn parses_generated_pdf() {
        let bytes = build_pdf("Hello Bluey PDF");
        match parse_document(&bytes, DocumentFormat::Pdf) {
            Ok(parsed) => assert!(
                parsed.text.contains("Hello Bluey PDF"),
                "expected extracted text, got: {:?}",
                parsed.text
            ),
            Err(e) => panic!("pdf-extract failed on the minimal PDF: {e}"),
        }
    }

    #[test]
    fn invalid_pdf_is_a_parse_error() {
        let err = parse_document(b"%PDF-1.4 garbage", DocumentFormat::Pdf).unwrap_err();
        assert_eq!(err.code, "storage.parse");
    }
}
