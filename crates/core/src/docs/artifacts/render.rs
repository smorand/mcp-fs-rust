//! The two renderings of a kept table (SPEC-0019 FR-NEW-002), byte exact, and
//! the full document view built from the conversion text (FR-NEW-007).
//!
//! Cells arrive normalized (trimmed, `\|` already read as `|`) from capture,
//! so both renderings show the same value; each walks the grid once, O(cells).

use crate::errors::{Result, ToolError};

/// A format `fs.get_table` answers in (FR-NEW-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableFormat {
    Markdown,
    Csv,
}

impl TableFormat {
    /// The format a caller asked for, `markdown` when none; matched case
    /// sensitively, so `CSV` and `md` are refused (FR-NEW-024).
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self> {
        match raw {
            None | Some("markdown") => Ok(Self::Markdown),
            Some("csv") => Ok(Self::Csv),
            Some(other) => Err(ToolError::invalid_argument(format!(
                "Unsupported format: {other} (use markdown or csv)"
            ))),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Csv => "csv",
        }
    }

    /// `cells` rendered in this format.
    pub(crate) fn render(self, cells: &[Vec<String>]) -> String {
        match self {
            Self::Markdown => table_markdown(cells),
            Self::Csv => table_csv(cells),
        }
    }
}

/// How a full document view renders each table (FR-NEW-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableMode {
    Markdown,
    CsvReference,
    Both,
}

impl TableMode {
    /// The mode a caller asked for, `markdown` when none; matched case
    /// sensitively (FR-NEW-025).
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self> {
        match raw {
            None | Some("markdown") => Ok(Self::Markdown),
            Some("csv-reference") => Ok(Self::CsvReference),
            Some("both") => Ok(Self::Both),
            Some(other) => Err(ToolError::invalid_argument(format!(
                "Unsupported table mode: {other} (use markdown, csv-reference or both)"
            ))),
        }
    }
}

/// A kept image as the view places it.
pub(crate) struct ViewImage {
    pub seq: i64,
    pub caption: String,
    /// 0 based line of its picture reference in the conversion text.
    pub line: usize,
}

/// A kept table as the view renders it.
pub(crate) struct ViewTable {
    pub seq: i64,
    pub caption: String,
    pub cells: Vec<Vec<String>>,
    /// 0 based first line of its block in the conversion text.
    pub line_start: usize,
    pub line_count: usize,
}

/// What the view does with one line of the conversion text.
enum LineAction<'a> {
    Keep,
    Drop,
    /// Write this image line, with this terminator (the one its removed
    /// block's last line carried), between empty lines (FR-NEW-036).
    Image(String, &'a str),
    /// Write this rendering, terminator included, instead of the line.
    Table(String),
}

/// The full document view (FR-NEW-007, FR-NEW-008): `text` with each kept
/// image's picture reference line replaced by its image line between empty
/// lines (and its caption line removed when it repeats the caption), and each
/// kept table's block replaced by its rendering in `mode`. Every other line is
/// copied byte for byte, line terminators included, so a set with nothing
/// kept views as its conversion text.
///
/// Lines are counted as `str::lines` counts them, the way `parse` recorded the
/// positions. A position outside the text (never written by capture) is
/// ignored rather than trusted. Time O(text + cells), space O(lines).
pub(crate) fn document_view(
    text: &str,
    images: &[ViewImage],
    tables: &[ViewTable],
    captions: bool,
    mode: TableMode,
) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    // The terminator synthetic blank lines are written with, matching the
    // text's own line ending style (FR-NEW-036).
    let line_sep = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut actions: Vec<LineAction<'_>> = lines.iter().map(|_| LineAction::Keep).collect();
    for t in tables {
        let end = t.line_start.saturating_add(t.line_count);
        if t.line_count == 0 || end > lines.len() {
            continue;
        }
        // The rendering takes the terminator of the block's last line, so a
        // block ending the text without one stays without one.
        let rendered = format!("{}{}", table_view(t, mode), terminator(lines[end - 1]));
        actions[t.line_start] = LineAction::Table(rendered);
        for a in &mut actions[t.line_start + 1..end] {
            *a = LineAction::Drop;
        }
    }
    for i in images {
        if !matches!(actions.get(i.line), Some(LineAction::Keep)) {
            continue;
        }
        // The matched caption line (FR-NEW-007), if any.
        let next =
            (i.line + 1..lines.len()).find(|&n| !content(lines[n]).trim_matches(' ').is_empty());
        let cap_line = next
            .filter(|&n| !i.caption.is_empty() && content(lines[n]) == i.caption)
            .filter(|&n| matches!(actions[n], LineAction::Keep));
        // Every empty line directly before, between or after the removed
        // lines is removed too (FR-NEW-036); nothing else is touched.
        let mut start = i.line;
        while start > 0
            && content(lines[start - 1]).trim_matches(' ').is_empty()
            && matches!(actions[start - 1], LineAction::Keep)
        {
            start -= 1;
        }
        let mut end = cap_line.unwrap_or(i.line);
        while end + 1 < lines.len()
            && content(lines[end + 1]).trim_matches(' ').is_empty()
            && matches!(actions[end + 1], LineAction::Keep)
        {
            end += 1;
        }
        let image_terminator = terminator(lines[end]);
        for a in &mut actions[start..=end] {
            *a = LineAction::Drop;
        }
        actions[i.line] = LineAction::Image(image_line(i, captions), image_terminator);
    }
    // Written in order, skipping drops; an empty line is owed before an
    // image unless it is the very first thing written, and one after unless
    // nothing follows, but two images back to back (nothing else between
    // them) share that single empty line rather than stacking two
    // (FR-NEW-036).
    let mut out = String::with_capacity(text.len());
    let mut wrote_anything = false;
    let mut after_image = false;
    for (idx, action) in actions.iter().enumerate() {
        match action {
            LineAction::Keep => {
                if after_image {
                    out.push_str(line_sep);
                }
                out.push_str(lines[idx]);
                after_image = false;
                wrote_anything = true;
            }
            LineAction::Drop => {}
            LineAction::Image(image, image_terminator) => {
                if wrote_anything {
                    out.push_str(line_sep);
                }
                out.push_str(image);
                out.push_str(image_terminator);
                after_image = true;
                wrote_anything = true;
            }
            LineAction::Table(rendered) => {
                if after_image {
                    out.push_str(line_sep);
                }
                out.push_str(rendered);
                after_image = false;
                wrote_anything = true;
            }
        }
    }
    out
}

/// The image line (FR-NEW-007, FR-NEW-008).
fn image_line(i: &ViewImage, captions: bool) -> String {
    if captions && !i.caption.is_empty() {
        format!("[Image image-{}: {}]", i.seq, i.caption)
    } else {
        format!("[Image image-{}]", i.seq)
    }
}

/// A table's rendering in `mode`, ended by nothing: the caller appends the
/// block's own last terminator.
fn table_view(t: &ViewTable, mode: TableMode) -> String {
    let reference = if t.caption.is_empty() {
        format!("[Table table-{} (CSV)]", t.seq)
    } else {
        format!("[Table table-{}: {} (CSV)]", t.seq, t.caption)
    };
    match mode {
        TableMode::Markdown => table_markdown(&t.cells),
        TableMode::CsvReference => reference,
        TableMode::Both => format!("{}\n{reference}", table_markdown(&t.cells)),
    }
}

/// A line without its terminator (`\n` or `\r\n`), as `str::lines` yields it.
fn content(line: &str) -> &str {
    match line.strip_suffix('\n') {
        Some(body) => body.strip_suffix('\r').unwrap_or(body),
        None => line,
    }
}

/// The terminator `line` ends with, empty for a last line without one.
fn terminator(line: &str) -> &str {
    &line[content(line).len()..]
}

/// The widest row's cell count, header included (FR-NEW-002).
pub(crate) fn width(cells: &[Vec<String>]) -> usize {
    cells.iter().map(Vec::len).max().unwrap_or(0)
}

/// The Markdown rendering of a table (FR-NEW-002): header, separator, one line
/// per data row, short rows padded, `|` escaped, each CR LF, lone CR or lone LF
/// as one space, no trailing newline. A header only table is two lines.
pub(crate) fn table_markdown(cells: &[Vec<String>]) -> String {
    let cols = width(cells);
    let mut out = String::new();
    let push_row = |out: &mut String, row: &[String]| {
        out.push('|');
        for i in 0..cols {
            out.push(' ');
            if let Some(cell) = row.get(i) {
                push_markdown_cell(out, cell);
            }
            out.push_str(" |");
        }
    };
    let Some((header, rows)) = cells.split_first() else {
        return out;
    };
    push_row(&mut out, header);
    out.push_str("\n|");
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    for row in rows {
        out.push('\n');
        push_row(&mut out, row);
    }
    out
}

/// One Markdown cell: a pipe escaped, a CR LF pair, lone CR or lone LF as one
/// space.
fn push_markdown_cell(out: &mut String, cell: &str) {
    let mut chars = cell.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '|' => out.push_str("\\|"),
            '\r' => {
                chars.next_if_eq(&'\n');
                out.push(' ');
            }
            '\n' => out.push(' '),
            _ => out.push(c),
        }
    }
}

/// The CSV rendering of a table (FR-NEW-002): comma separated, a field quoted
/// only when it holds a comma, a double quote, a CR or a LF, inner quotes
/// doubled, LF between lines, no trailing newline, short rows padded.
pub(crate) fn table_csv(cells: &[Vec<String>]) -> String {
    let cols = width(cells);
    let mut out = String::new();
    for (r, row) in cells.iter().enumerate() {
        if r > 0 {
            out.push('\n');
        }
        for i in 0..cols {
            if i > 0 {
                out.push(',');
            }
            if let Some(cell) = row.get(i) {
                push_csv_field(&mut out, cell);
            }
        }
    }
    out
}

fn push_csv_field(out: &mut String, cell: &str) {
    if !cell.contains([',', '"', '\r', '\n']) {
        out.push_str(cell);
        return;
    }
    out.push('"');
    out.push_str(&cell.replace('"', "\"\""));
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|r| r.iter().map(|c| (*c).to_string()).collect()).collect()
    }

    #[test]
    fn markdown_pads_escapes_and_flattens_newlines() {
        let t = cells(&[&["h1", "h2"], &["a|b", "x\r\ny\rz\nw"], &["c"]]);
        assert_eq!(table_markdown(&t), "| h1 | h2 |\n| --- | --- |\n| a\\|b | x y z w |\n| c |  |");
        assert_eq!(table_markdown(&cells(&[&["h"]])), "| h |\n| --- |");
    }

    #[test]
    fn csv_quotes_only_when_needed_and_pads() {
        let t = cells(&[&["h1", "h2"], &["a,b", "say \"hi\""], &["line\nbreak"]]);
        assert_eq!(table_csv(&t), "h1,h2\n\"a,b\",\"say \"\"hi\"\"\"\n\"line\nbreak\",");
    }

    #[test]
    fn empty_and_header_only_tables() {
        assert_eq!(table_markdown(&[]), "");
        assert_eq!(table_csv(&[]), "");
        let t = cells(&[&["A", "B"]]);
        assert_eq!(table_markdown(&t), "| A | B |\n| --- | --- |");
        assert_eq!(table_csv(&t), "A,B");
    }

    #[test]
    fn csv_keeps_a_cr_lf_inside_quotes() {
        assert_eq!(table_csv(&cells(&[&["v"], &["a\r\nb"]])), "v\n\"a\r\nb\"");
        assert_eq!(table_csv(&cells(&[&["v"], &["a\rb"]])), "v\n\"a\rb\"");
    }

    #[test]
    fn formats_are_matched_case_sensitively() {
        assert_eq!(TableFormat::parse(None).unwrap(), TableFormat::Markdown);
        assert_eq!(TableFormat::parse(Some("markdown")).unwrap(), TableFormat::Markdown);
        assert_eq!(TableFormat::parse(Some("csv")).unwrap(), TableFormat::Csv);
        for bad in ["CSV", "md", "xml", "", "Markdown"] {
            let err = TableFormat::parse(Some(bad)).unwrap_err();
            assert_eq!(err.message, format!("Unsupported format: {bad} (use markdown or csv)"));
        }
    }

    fn table(seq: i64, caption: &str, line_start: usize, line_count: usize) -> ViewTable {
        ViewTable {
            seq,
            caption: caption.into(),
            cells: cells(&[&["a"], &["1"]]),
            line_start,
            line_count,
        }
    }

    #[test]
    fn table_modes_are_matched_case_sensitively() {
        assert_eq!(TableMode::parse(None).unwrap(), TableMode::Markdown);
        assert_eq!(TableMode::parse(Some("csv-reference")).unwrap(), TableMode::CsvReference);
        assert_eq!(TableMode::parse(Some("both")).unwrap(), TableMode::Both);
        for bad in ["html", "CSV-REFERENCE", "markdown,both", "", "Both"] {
            let err = TableMode::parse(Some(bad)).unwrap_err();
            assert_eq!(
                err.message,
                format!("Unsupported table mode: {bad} (use markdown, csv-reference or both)")
            );
        }
    }

    #[test]
    fn a_view_with_nothing_kept_is_the_text_byte_for_byte() {
        for text in ["", "a", "a\r\nb\r\n", "x\n\n| a |\n| --- |\n", "no end"] {
            assert_eq!(document_view(text, &[], &[], true, TableMode::Both), text);
        }
    }

    #[test]
    fn a_table_keeps_its_last_terminator_and_crlf_lines_stay() {
        let text = "x\r\n| a |\r\n| --- |\r\n| 1 |\r\ny\r\n";
        let t = [table(1, "", 1, 3)];
        assert_eq!(
            document_view(text, &[], &t, true, TableMode::Markdown),
            "x\r\n| a |\n| --- |\n| 1 |\r\ny\r\n"
        );
        let text = "x\n| a |\n| --- |\n| 1 |";
        assert_eq!(
            document_view(text, &[], &t, true, TableMode::Both),
            "x\n| a |\n| --- |\n| 1 |\n[Table table-1 (CSV)]"
        );
    }

    #[test]
    fn an_image_line_replaces_the_reference_and_a_repeated_caption() {
        // Updated for FR-NEW-036 (SPEC-0019/US-0009): the blank line between
        // the ref and the removed caption no longer survives alongside the
        // synthetic one, so exactly one empty line stands on each side.
        let text = "a\n![](p.png)\n\nCap\nCap\n";
        let img = |caption: &str| ViewImage { seq: 3, caption: caption.into(), line: 1 };
        assert_eq!(
            document_view(text, &[img("Cap")], &[], true, TableMode::Markdown),
            "a\n\n[Image image-3: Cap]\n\nCap\n"
        );
        assert_eq!(
            document_view(text, &[img("Cap")], &[], false, TableMode::Markdown),
            "a\n\n[Image image-3]\n\nCap\n"
        );
        // An empty caption matches no line, not even an empty one.
        let first = ViewImage { seq: 1, caption: String::new(), line: 0 };
        assert_eq!(
            document_view("![](p.png)\n\nx\n", &[first], &[], true, TableMode::Markdown),
            "[Image image-1]\n\nx\n"
        );
    }

    #[test]
    fn an_image_collapses_surrounding_empty_lines_to_one_each_side() {
        let text = "Intro\n\n![](p.png)\n\nFigure 1: Revenue 2024\n\nBody text";
        let img = ViewImage { seq: 1, caption: "Figure 1: Revenue 2024".into(), line: 2 };
        assert_eq!(
            document_view(text, &[img], &[], true, TableMode::Markdown),
            "Intro\n\n[Image image-1: Figure 1: Revenue 2024]\n\nBody text"
        );
    }

    #[test]
    fn an_image_as_the_first_line_has_no_leading_empty_line() {
        let text = "![](p.png)\n\nBody text";
        let img = ViewImage { seq: 1, caption: String::new(), line: 0 };
        assert_eq!(
            document_view(text, &[img], &[], true, TableMode::Markdown),
            "[Image image-1]\n\nBody text"
        );
    }

    #[test]
    fn an_image_as_the_last_line_has_no_trailing_empty_line() {
        let text = "Intro\n\n\n![](p.png)";
        let img = ViewImage { seq: 1, caption: String::new(), line: 3 };
        assert_eq!(
            document_view(text, &[img], &[], true, TableMode::Markdown),
            "Intro\n\n[Image image-1]"
        );
    }

    #[test]
    fn positions_outside_the_text_are_ignored() {
        let text = "a\nb\n";
        let images = [ViewImage { seq: 1, caption: String::new(), line: usize::MAX }];
        let tables = [table(1, "", 1, 5), table(2, "", usize::MAX, 2), table(3, "", 0, 0)];
        assert_eq!(document_view(text, &images, &tables, true, TableMode::Both), text);
    }

    #[test]
    fn adjacent_images_have_exactly_one_blank_line_between() {
        let text = "x\n![](a)\n![](b)\ny\n";
        let imgs = [
            ViewImage { seq: 1, caption: String::new(), line: 1 },
            ViewImage { seq: 2, caption: String::new(), line: 2 },
        ];
        assert_eq!(
            document_view(text, &imgs, &[], true, TableMode::Markdown),
            "x\n\n[Image image-1]\n\n[Image image-2]\n\ny\n"
        );
    }

    #[test]
    fn adjacent_images_with_a_blank_line_between_still_get_exactly_one() {
        let text = "x\n![](a)\n\n![](b)\ny\n";
        let imgs = [
            ViewImage { seq: 1, caption: String::new(), line: 1 },
            ViewImage { seq: 2, caption: String::new(), line: 3 },
        ];
        assert_eq!(
            document_view(text, &imgs, &[], true, TableMode::Markdown),
            "x\n\n[Image image-1]\n\n[Image image-2]\n\ny\n"
        );
    }

    #[test]
    fn an_image_directly_before_a_table_gets_its_blank_lines() {
        let text = "x\n![](a)\n| h |\n| --- |\n| 1 |\ny\n";
        let img = ViewImage { seq: 1, caption: String::new(), line: 1 };
        let t = table(1, "", 2, 3);
        assert_eq!(
            document_view(text, &[img], &[t], true, TableMode::Markdown),
            "x\n\n[Image image-1]\n\n| a |\n| --- |\n| 1 |\ny\n"
        );
    }

    #[test]
    fn a_tab_only_line_next_to_an_image_is_not_treated_as_empty() {
        let text = "a\n![](p.png)\n\t\nb\n";
        let img = ViewImage { seq: 1, caption: String::new(), line: 1 };
        assert_eq!(
            document_view(text, &[img], &[], true, TableMode::Markdown),
            "a\n\n[Image image-1]\n\n\t\nb\n"
        );
    }

    #[test]
    fn crlf_text_gets_crlf_blank_lines_around_an_image() {
        let text = "Intro\r\n![](p.png)\r\nBody\r\n";
        let img = ViewImage { seq: 1, caption: String::new(), line: 1 };
        assert_eq!(
            document_view(text, &[img], &[], true, TableMode::Markdown),
            "Intro\r\n\r\n[Image image-1]\r\n\r\nBody\r\n"
        );
    }
}
