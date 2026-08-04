//! Markdown to `.docx` (WordprocessingML). Port of the C# `Core/DocxWriter.cs`.
//!
//! The C# leans on the OpenXML SDK. Here the package is assembled by hand with
//! the `zip` crate already in the tree: a Word document only needs four parts
//! plus a stylesheet, so pulling a heavyweight OOXML dependency (and its
//! transitive XML stack) would buy nothing.
//!
//! Supported Markdown subset, matching the C# plus the two additions noted below:
//! ATX headings `#`..`######`, paragraphs, bullet lists, numbered lists, GitHub
//! pipe tables, fenced code blocks, and inline `**bold**` / `*italic*`.
//! Everything else degrades to plain text.
//!
//! Deliberate deviations from the C#, both improvements:
//!   1. numbered list items keep their marker (the C# dropped it, which silently
//!      lost the ordering information),
//!   2. fenced code blocks render as monospaced, indentation preserving
//!      paragraphs instead of leaking the ``` fence lines into the body.

use crate::errors::Result;
use crate::util::text::split_lines;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// Render Markdown into `.docx` bytes. `title` becomes a leading `Title` styled
/// paragraph, exactly like the C#.
pub fn render_markdown_to_docx(markdown: &str, title: Option<&str>) -> Result<Vec<u8>> {
    let body = build_body(markdown, title);
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W_NS}"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:bottom="1134" w:left="1134" w:right="1134"/></w:sectPr></w:body></w:document>"#
    );
    pack(&document)
}

fn pack(document_xml: &str) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    // Deflate everywhere: Word accepts it and the XML compresses ~10x.
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("word/_rels/document.xml.rels", DOCUMENT_RELS),
        ("word/document.xml", document_xml),
        ("word/styles.xml", STYLES),
    ] {
        zip.start_file(name, opts)
            .map_err(|e| crate::errors::ToolError::internal(format!("docx zip: {e}")))?;
        zip.write_all(content.as_bytes())?;
    }
    let cursor = zip
        .finish()
        .map_err(|e| crate::errors::ToolError::internal(format!("docx zip finish: {e}")))?;
    Ok(cursor.into_inner())
}

// ── markdown walk ────────────────────────────────────────────────────────────

fn build_body(markdown: &str, title: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(t) = title.filter(|t| !t.is_empty()) {
        out.push_str(&heading_paragraph(t, 0));
    }
    let lines = split_lines(markdown);
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        let stripped = line.trim();
        if stripped.is_empty() {
            i += 1;
            continue;
        }

        if let Some(fence) = fence_marker(stripped) {
            i += 1;
            let mut code = Vec::new();
            while i < lines.len() && fence_marker(lines[i].trim()) != Some(fence) {
                code.push(lines[i].clone());
                i += 1;
            }
            if i < lines.len() {
                i += 1; // consume the closing fence
            }
            out.push_str(&code_block(&code));
            continue;
        }

        if let Some((level, text)) = heading_of(line) {
            out.push_str(&heading_paragraph(text.trim(), level));
            i += 1;
            continue;
        }

        if is_table_row(line) && i + 1 < lines.len() && is_table_separator(&lines[i + 1]) {
            let header = split_row(line);
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && is_table_row(&lines[i]) {
                rows.push(split_row(&lines[i]));
                i += 1;
            }
            out.push_str(&build_table(&header, &rows));
            continue;
        }

        if let Some(text) = bullet_of(line) {
            out.push_str(&list_paragraph(text.trim(), "\u{2022} "));
            i += 1;
            continue;
        }

        if let Some((marker, text)) = numbered_of(line) {
            // Keep the marker: a numbered list without numbers is not a numbered list.
            out.push_str(&list_paragraph(text.trim(), &format!("{marker} ")));
            i += 1;
            continue;
        }

        out.push_str(&paragraph(stripped));
        i += 1;
    }
    out
}

/// `Some(fence_char)` when the line opens or closes a fenced code block.
fn fence_marker(stripped: &str) -> Option<char> {
    for c in ['`', '~'] {
        if stripped.starts_with(&format!("{c}{c}{c}")) {
            return Some(c);
        }
    }
    None
}

fn heading_of(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    // ATX requires whitespace between the hashes and the text.
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some((hashes, rest))
}

fn bullet_of(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let mut chars = t.chars();
    let marker = chars.next()?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }
    let rest = &t[marker.len_utf8()..];
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim_start())
}

/// `Some(("1.", "text"))` for `1. text` and `1) text`.
fn numbered_of(line: &str) -> Option<(String, &str)> {
    let t = line.trim_start();
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = &t[digits.len()..];
    let sep = rest.chars().next()?;
    if !matches!(sep, '.' | ')') {
        return None;
    }
    let after = &rest[1..];
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    Some((format!("{digits}{sep}"), after.trim_start()))
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && t.starts_with('|') && t.ends_with('|')
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty()
        && t.contains('-')
        && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn split_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

// ── WordprocessingML fragments ───────────────────────────────────────────────

fn heading_paragraph(text: &str, level: usize) -> String {
    let style = if level == 0 { "Title".to_string() } else { format!("Heading{}", level.min(6)) };
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr>{}</w:p>"#,
        inline_runs(text)
    )
}

fn list_paragraph(text: &str, prefix: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="ListParagraph"/></w:pPr>{}</w:p>"#,
        inline_runs(&format!("{prefix}{text}"))
    )
}

fn paragraph(text: &str) -> String {
    format!("<w:p>{}</w:p>", inline_runs(text))
}

fn code_block(lines: &[String]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&format!(
            r#"<w:p><w:pPr><w:pStyle w:val="SourceCode"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:cs="Consolas"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
            escape(line)
        ));
    }
    if out.is_empty() {
        // An empty fenced block still deserves a paragraph, so the surrounding
        // text does not visually collapse together.
        out.push_str("<w:p/>");
    }
    out
}

fn build_table(header: &[String], rows: &[Vec<String>]) -> String {
    let width = header.len();
    let mut out = String::from(
        r#"<w:tbl><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4"/><w:bottom w:val="single" w:sz="4"/><w:left w:val="single" w:sz="4"/><w:right w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="4"/><w:insideV w:val="single" w:sz="4"/></w:tblBorders></w:tblPr>"#,
    );
    out.push_str(&build_row(header, width));
    for row in rows {
        out.push_str(&build_row(row, width));
    }
    out.push_str("</w:tbl>");
    out
}

fn build_row(cells: &[String], width: usize) -> String {
    let mut out = String::from("<w:tr>");
    for i in 0..width {
        let value = cells.get(i).map(String::as_str).unwrap_or("");
        out.push_str(&format!(
            "<w:tc><w:tcPr><w:tcW w:w=\"0\" w:type=\"auto\"/></w:tcPr><w:p>{}</w:p></w:tc>",
            inline_runs(value)
        ));
    }
    out.push_str("</w:tr>");
    out
}

/// Inline markup: `**bold**` wins over `*italic*`, and unpaired markers stay
/// literal. Hand rolled because the `regex` crate has no lookaround, which is
/// what the C# italic pattern relied on.
fn inline_runs(text: &str) -> String {
    let mut out = String::new();
    let bytes = text.as_bytes();
    let mut plain = String::new();
    let mut i = 0;
    while i < text.len() {
        if bytes[i] == b'*' {
            let double = i + 1 < text.len() && bytes[i + 1] == b'*';
            let marker = if double { "**" } else { "*" };
            if let Some(end) = find_closing(text, i + marker.len(), marker) {
                let inner = &text[i + marker.len()..end];
                if !inner.is_empty() {
                    if !plain.is_empty() {
                        out.push_str(&run(&std::mem::take(&mut plain), false, false));
                    }
                    out.push_str(&run(inner, double, !double));
                    i = end + marker.len();
                    continue;
                }
            }
        }
        // Not a usable marker: keep the byte as literal text.
        let ch = text[i..].chars().next().expect("valid char boundary");
        plain.push(ch);
        i += ch.len_utf8();
    }
    if !plain.is_empty() {
        out.push_str(&run(&plain, false, false));
    }
    if out.is_empty() {
        out.push_str(&run("", false, false));
    }
    out
}

/// Index of the closing `marker` at or after `from`, refusing a `*` close that is
/// really the start of a `**` pair.
fn find_closing(text: &str, from: usize, marker: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < text.len() {
        if bytes[i] == b'*' {
            let double = i + 1 < text.len() && bytes[i + 1] == b'*';
            if marker == "**" && double {
                return Some(i);
            }
            if marker == "*" && !double {
                return Some(i);
            }
            // Skip the whole run of stars we could not use.
            while i < text.len() && bytes[i] == b'*' {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    None
}

fn run(text: &str, bold: bool, italic: bool) -> String {
    let mut props = String::new();
    if bold || italic {
        props.push_str("<w:rPr>");
        if bold {
            props.push_str("<w:b/>");
        }
        if italic {
            props.push_str("<w:i/>");
        }
        props.push_str("</w:rPr>");
    }
    format!(
        r#"<w:r>{props}<w:t xml:space="preserve">{}</w:t></w:r>"#,
        escape(text)
    )
}

/// XML text escaping. `>` is escaped too: it is not strictly required in content,
/// but it keeps the output byte identical whatever the consumer.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are illegal in XML 1.0 and make Word refuse the
            // file, so they are dropped rather than escaped.
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// Minimal stylesheet giving Title, Heading1..6, ListParagraph and SourceCode a
/// real look. Without it Word renders every paragraph identically.
const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:cs="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:before="240" w:after="240"/></w:pPr><w:rPr><w:b/><w:sz w:val="56"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:outlineLvl w:val="0"/><w:spacing w:before="240" w:after="120"/></w:pPr><w:rPr><w:b/><w:sz w:val="40"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:outlineLvl w:val="1"/><w:spacing w:before="200" w:after="120"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:outlineLvl w:val="2"/><w:spacing w:before="180" w:after="120"/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading4"><w:name w:val="heading 4"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:outlineLvl w:val="3"/></w:pPr><w:rPr><w:b/><w:sz w:val="26"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading5"><w:name w:val="heading 5"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:outlineLvl w:val="4"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading6"><w:name w:val="heading 6"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:outlineLvl w:val="5"/></w:pPr><w:rPr><w:b/><w:i/><w:sz w:val="24"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:ind w:left="720"/><w:spacing w:after="60"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="SourceCode"><w:name w:val="Source Code"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="0"/></w:pPr><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:cs="Consolas"/><w:sz w:val="20"/></w:rPr></w:style></w:styles>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn parts(bytes: &[u8]) -> std::collections::BTreeMap<String, String> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).expect("valid zip");
        let mut out = std::collections::BTreeMap::new();
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut s = String::new();
            f.read_to_string(&mut s).unwrap();
            out.insert(name, s);
        }
        out
    }

    /// Full pull-parse of the XML: any imbalance or bad escape fails here, which
    /// is the property Word actually cares about.
    fn assert_well_formed(xml: &str) {
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut depth: i64 = 0;
        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Start(_)) => depth += 1,
                Ok(quick_xml::events::Event::End(_)) => depth -= 1,
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("not well formed XML: {e}"),
            }
            assert!(depth >= 0, "unbalanced end tag");
        }
        assert_eq!(depth, 0, "unbalanced XML tree");
    }

    #[test]
    fn produces_the_required_ooxml_parts() {
        let bytes = render_markdown_to_docx("# Hello\n\nbody\n", None).unwrap();
        let p = parts(&bytes);
        for required in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "word/styles.xml",
        ] {
            assert!(p.contains_key(required), "missing part {required}");
        }
    }

    #[test]
    fn every_part_is_well_formed_xml() {
        let md = "# H1\n\nText with **bold** and *italic* & an <angle> \"quote\".\n\n- a\n- b\n\n1. one\n2. two\n\n```rust\nfn main() {}\n```\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let bytes = render_markdown_to_docx(md, Some("My & Title")).unwrap();
        for (name, xml) in parts(&bytes) {
            assert_well_formed(&xml);
            assert!(xml.starts_with("<?xml"), "{name} misses the XML declaration");
        }
    }

    #[test]
    fn heading_text_appears_with_the_right_style() {
        let bytes = render_markdown_to_docx("### Third Level\n", None).unwrap();
        let doc = parts(&bytes)["word/document.xml"].clone();
        assert!(doc.contains(r#"<w:pStyle w:val="Heading3"/>"#), "{doc}");
        assert!(doc.contains("Third Level"));
    }

    #[test]
    fn all_six_heading_levels_map_to_styles() {
        for level in 1..=6usize {
            let md = format!("{} Level {level}\n", "#".repeat(level));
            let doc = parts(&render_markdown_to_docx(&md, None).unwrap())["word/document.xml"].clone();
            assert!(doc.contains(&format!(r#"<w:pStyle w:val="Heading{level}"/>"#)), "level {level}");
        }
    }

    #[test]
    fn seven_hashes_is_not_a_heading() {
        let doc = parts(&render_markdown_to_docx("####### too deep\n", None).unwrap())["word/document.xml"].clone();
        assert!(!doc.contains("w:pStyle"), "should be a plain paragraph: {doc}");
        assert!(doc.contains("####### too deep"));
    }

    #[test]
    fn title_becomes_a_title_paragraph() {
        let doc = parts(&render_markdown_to_docx("paragraph one\n", Some("The Title")).unwrap())
            ["word/document.xml"]
            .clone();
        assert!(doc.contains(r#"<w:pStyle w:val="Title"/>"#));
        assert!(doc.contains("The Title"));
        // the title comes first, before the body text
        assert!(doc.find("The Title").unwrap() < doc.find("paragraph one").unwrap());
    }

    #[test]
    fn empty_title_is_ignored() {
        let doc = parts(&render_markdown_to_docx("para\n", Some("")).unwrap())["word/document.xml"].clone();
        assert!(!doc.contains("Title"));
    }

    #[test]
    fn bold_and_italic_become_run_properties() {
        let doc = parts(&render_markdown_to_docx("a **strong** and *slanted* end\n", None).unwrap())
            ["word/document.xml"]
            .clone();
        assert!(doc.contains("<w:rPr><w:b/></w:rPr>"), "{doc}");
        assert!(doc.contains("<w:rPr><w:i/></w:rPr>"), "{doc}");
        assert!(doc.contains(">strong<"));
        assert!(doc.contains(">slanted<"));
    }

    #[test]
    fn unpaired_star_stays_literal() {
        let doc = parts(&render_markdown_to_docx("2 * 3 = 6\n", None).unwrap())["word/document.xml"].clone();
        assert!(doc.contains("2 * 3 = 6"), "{doc}");
        assert!(!doc.contains("<w:i/>"));
    }

    #[test]
    fn bullet_list_gets_the_bullet_glyph() {
        let doc = parts(&render_markdown_to_docx("- first\n* second\n+ third\n", None).unwrap())
            ["word/document.xml"]
            .clone();
        assert_eq!(doc.matches(r#"<w:pStyle w:val="ListParagraph"/>"#).count(), 3);
        assert_eq!(doc.matches('\u{2022}').count(), 3);
        assert!(doc.contains("first") && doc.contains("second") && doc.contains("third"));
    }

    #[test]
    fn numbered_list_keeps_its_marker() {
        let doc = parts(&render_markdown_to_docx("1. alpha\n2) beta\n", None).unwrap())
            ["word/document.xml"]
            .clone();
        assert_eq!(doc.matches(r#"<w:pStyle w:val="ListParagraph"/>"#).count(), 2);
        assert!(doc.contains("1. alpha"), "{doc}");
        assert!(doc.contains("2) beta"), "{doc}");
    }

    #[test]
    fn code_block_uses_monospace_and_drops_the_fences() {
        let md = "```python\nx = 1\nif x:\n    pass\n```\n";
        let doc = parts(&render_markdown_to_docx(md, None).unwrap())["word/document.xml"].clone();
        assert!(!doc.contains("```"), "fences must not leak: {doc}");
        assert!(!doc.contains(">python<"), "info string must not become text");
        assert!(doc.contains("Consolas"));
        assert!(doc.contains("    pass"), "indentation preserved: {doc}");
        assert_eq!(doc.matches(r#"<w:pStyle w:val="SourceCode"/>"#).count(), 3);
    }

    #[test]
    fn tilde_fences_work_too() {
        let doc = parts(&render_markdown_to_docx("~~~\nraw\n~~~\n", None).unwrap())["word/document.xml"].clone();
        assert!(!doc.contains("~~~"));
        assert!(doc.contains(">raw<"));
    }

    #[test]
    fn unclosed_fence_consumes_the_rest() {
        let doc = parts(&render_markdown_to_docx("```\nline\n", None).unwrap())["word/document.xml"].clone();
        assert!(doc.contains(">line<"));
        assert!(!doc.contains("```"));
        assert_well_formed(&doc);
    }

    #[test]
    fn pipe_table_becomes_a_word_table() {
        let md = "| Name | Qty |\n| --- | ---: |\n| bolt | 3 |\n| nut | 12 |\n";
        let doc = parts(&render_markdown_to_docx(md, None).unwrap())["word/document.xml"].clone();
        assert!(doc.contains("<w:tbl>"));
        assert_eq!(doc.matches("<w:tr>").count(), 3, "header plus two rows");
        assert_eq!(doc.matches("<w:tc>").count(), 6);
        assert!(doc.contains(">bolt<") && doc.contains(">12<"));
    }

    #[test]
    fn a_pipe_line_without_separator_is_a_paragraph() {
        let doc = parts(&render_markdown_to_docx("| not | a table |\n", None).unwrap())
            ["word/document.xml"]
            .clone();
        assert!(!doc.contains("<w:tbl>"));
        assert!(doc.contains("| not | a table |"));
    }

    #[test]
    fn short_rows_are_padded_to_the_header_width() {
        let md = "| a | b | c |\n| - | - | - |\n| 1 |\n";
        let doc = parts(&render_markdown_to_docx(md, None).unwrap())["word/document.xml"].clone();
        // 3 header cells plus 3 body cells even though only one value was given
        assert_eq!(doc.matches("<w:tc>").count(), 6);
    }

    #[test]
    fn special_characters_are_escaped() {
        let doc = parts(&render_markdown_to_docx("a < b & c > d \"q\" 'r'\n", None).unwrap())
            ["word/document.xml"]
            .clone();
        assert!(doc.contains("&lt;") && doc.contains("&amp;") && doc.contains("&gt;"));
        assert!(!doc.contains("a < b"));
        assert_well_formed(&doc);
    }

    #[test]
    fn control_characters_are_dropped() {
        let doc = parts(&render_markdown_to_docx("bad\u{0007}char\n", None).unwrap())
            ["word/document.xml"]
            .clone();
        assert!(doc.contains("badchar"), "{doc}");
        assert_well_formed(&doc);
    }

    #[test]
    fn empty_markdown_still_yields_a_valid_document() {
        let bytes = render_markdown_to_docx("", None).unwrap();
        let p = parts(&bytes);
        assert_well_formed(&p["word/document.xml"]);
        assert!(p["word/document.xml"].contains("<w:body>"));
        assert!(p["word/document.xml"].contains("<w:sectPr>"));
    }

    #[test]
    fn unicode_survives_the_round_trip() {
        let doc = parts(&render_markdown_to_docx("caf\u{e9} \u{4e2d}\u{6587} \u{1f600}\n", None).unwrap())
            ["word/document.xml"]
            .clone();
        assert!(doc.contains("caf\u{e9}"));
        assert!(doc.contains("\u{4e2d}\u{6587}"));
        assert!(doc.contains("\u{1f600}"));
    }

    #[test]
    fn content_types_declares_both_overrides() {
        let ct = parts(&render_markdown_to_docx("x", None).unwrap())["[Content_Types].xml"].clone();
        assert!(ct.contains("/word/document.xml"));
        assert!(ct.contains("/word/styles.xml"));
        assert!(ct.contains(r#"Extension="rels""#));
    }

    #[test]
    fn relationships_point_at_the_right_targets() {
        let p = parts(&render_markdown_to_docx("x", None).unwrap());
        assert!(p["_rels/.rels"].contains(r#"Target="word/document.xml""#));
        assert!(p["word/_rels/document.xml.rels"].contains(r#"Target="styles.xml""#));
    }
}
