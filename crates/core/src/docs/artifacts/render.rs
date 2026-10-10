//! The two renderings of a kept table (SPEC-0019 FR-NEW-002), byte exact.
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
}
