//! What the external converter's text yields (SPEC-0019 FR-NEW-033): the
//! picture references and the pipe table blocks, in reading order, with the
//! ids FR-NEW-001 assigns before anything is kept.

/// A line whose only content is one Markdown image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PictureRef {
    /// The n of `image-n`.
    pub seq: i64,
    pub alt: String,
    /// As written between the parentheses.
    pub target: String,
    /// 0 based line of the reference in the conversion text.
    pub line: usize,
}

/// A pipe table block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PipeTable {
    /// The n of `table-n`.
    pub seq: i64,
    pub caption: String,
    /// Header then data rows, cells trimmed and `\|` read as `|`; `None` when
    /// the separator's cell count differs from the header's (malformed).
    pub cells: Option<Vec<Vec<String>>>,
    /// 0 based first line of the block in the conversion text.
    pub line_start: usize,
    pub line_count: usize,
}

/// Every item of one conversion text, each list in reading order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub pictures: Vec<PictureRef>,
    pub tables: Vec<PipeTable>,
}

/// The fence that opens and closes a code block.
const FENCE: &str = "```";

/// Read `text` with the FR-NEW-033 rules, in one pass over its lines.
///
/// Time O(text length), space O(lines). Lines are split by `str::lines`, so a
/// CR LF end is one line end, as `fs.read` shows it.
pub(crate) fn parse(text: &str) -> Parsed {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Parsed::default();
    let mut in_fence = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with(FENCE) {
            in_fence = !in_fence;
            i += 1;
            continue;
        }
        if in_fence {
            i += 1;
            continue;
        }
        if let Some((alt, target)) = picture_reference(line) {
            let seq = out.pictures.len() as i64 + 1;
            out.pictures.push(PictureRef { seq, alt, target, line: i });
            i += 1;
            continue;
        }
        match table_at(&lines, i) {
            Some(mut table) => {
                table.seq = out.tables.len() as i64 + 1;
                table.caption = caption_before(&lines, i);
                i += table.line_count;
                out.tables.push(table);
            }
            None => i += 1,
        }
    }
    out
}

/// `(alt, target)` when the trimmed line is exactly one `![alt](target)`.
fn picture_reference(line: &str) -> Option<(String, String)> {
    let body = line.trim().strip_prefix("![")?.strip_suffix(')')?;
    let (alt, target) = body.split_once("](")?;
    // A second image on the line, or brackets that do not pair as one image,
    // make it ordinary text.
    if alt.contains(']') || target.contains("](") || target.contains("![") {
        return None;
    }
    Some((alt.to_string(), target.to_string()))
}

/// The block whose header is line `start`, malformed or not; `None` when the
/// line does not open a pipe table block.
fn table_at(lines: &[&str], start: usize) -> Option<PipeTable> {
    let header = pipe_cells(lines[start])?;
    let separator = lines.get(start + 1).and_then(|l| pipe_cells(l))?;
    if !is_separator(&separator) {
        return None;
    }
    let rows: Vec<Vec<&str>> = lines[start + 2..].iter().map_while(|l| pipe_cells(l)).collect();
    let line_count = 2 + rows.len();
    let cells = (separator.len() == header.len()).then(|| {
        std::iter::once(header)
            .chain(rows)
            .map(|r| r.iter().map(|c| cell_value(c)).collect())
            .collect()
    });
    Some(PipeTable { seq: 0, caption: String::new(), cells, line_start: start, line_count })
}

/// The raw cells of a line whose first non space character is `|`: one
/// leading `|` and one trailing unescaped `|` removed, then split on every `|`
/// not preceded by `\`.
fn pipe_cells(line: &str) -> Option<Vec<&str>> {
    let body = line.trim().strip_prefix('|')?;
    let body = match body.strip_suffix('|') {
        Some(inner) if !inner.ends_with('\\') => inner,
        _ => body,
    };
    let mut cells = Vec::new();
    let mut from = 0;
    let bytes = body.as_bytes();
    for (at, b) in bytes.iter().enumerate() {
        if *b == b'|' && (at == 0 || bytes[at - 1] != b'\\') {
            cells.push(&body[from..at]);
            from = at + 1;
        }
    }
    cells.push(&body[from..]);
    Some(cells)
}

/// Only `|`, `-`, `:` and spaces, with at least one `-` per cell.
fn is_separator(cells: &[&str]) -> bool {
    cells.iter().all(|c| c.contains('-') && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

/// A cell's value (FR-NEW-002): trimmed, each `\|` read as `|`.
fn cell_value(raw: &str) -> String {
    raw.trim().replace("\\|", "|")
}

/// `<text>` when the first non empty line before `start` reads
/// `Table <number>: <text>`, otherwise empty.
fn caption_before(lines: &[&str], start: usize) -> String {
    let Some(line) = lines[..start].iter().rev().find(|l| !l.trim().is_empty()) else {
        return String::new();
    };
    let Some(rest) = line.trim().strip_prefix("Table ") else {
        return String::new();
    };
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    match rest[digits..].strip_prefix(": ") {
        Some(text) if digits > 0 => text.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(rows: &[&[&str]]) -> Option<Vec<Vec<String>>> {
        Some(rows.iter().map(|r| r.iter().map(|c| (*c).to_string()).collect()).collect())
    }

    #[test]
    fn a_header_and_separator_make_a_table_with_its_data_rows() {
        let p = parse("intro\n| a | b |\n| --- | :-: |\n| 1 | 2 |\n|3|4|\nafter");
        assert_eq!(
            p.tables,
            vec![PipeTable {
                seq: 1,
                caption: String::new(),
                cells: cells(&[&["a", "b"], &["1", "2"], &["3", "4"]]),
                line_start: 1,
                line_count: 4,
            }]
        );
        assert!(p.pictures.is_empty());
    }

    #[test]
    fn the_caption_comes_from_a_table_line_before_the_block() {
        let p = parse("Table 1: Q1 sales\n\n| a |\n| --- |\n");
        assert_eq!(p.tables[0].caption, "Q1 sales");
        // E2E-128: any other line gives no caption.
        let p = parse("Summary\n| a |\n| --- |\n");
        assert_eq!(p.tables[0].caption, "");
        for not_a_caption in ["Table: x", "Table x: y", "Tables 1: x"] {
            let p = parse(&format!("{not_a_caption}\n| a |\n| --- |\n"));
            assert_eq!(p.tables[0].caption, "", "{not_a_caption}");
        }
    }

    #[test]
    fn a_separator_with_another_cell_count_is_a_malformed_table_keeping_its_id() {
        let text =
            "| a |\n| --- |\n\n| a | b |\n| --- | --- | --- |\n| 1 | 2 |\n\n| c |\n| --- |\n";
        let p = parse(text);
        let seqs: Vec<_> = p.tables.iter().map(|t| (t.seq, t.cells.is_some())).collect();
        assert_eq!(seqs, vec![(1, true), (2, false), (3, true)]);
        assert_eq!(p.tables[1].line_count, 3, "the malformed block still spans its rows");
    }

    #[test]
    fn an_escaped_pipe_is_not_a_cell_boundary() {
        // E2E-149: the header has 2 cells, the separator 3.
        let p = parse("| a\\|b | c |\n| --- | --- | --- |\n");
        assert_eq!(p.tables.len(), 1);
        assert_eq!(p.tables[0].cells, None);
        let p = parse("| a\\|b | c |\n| --- | --- |\n");
        assert_eq!(p.tables[0].cells, cells(&[&["a|b", "c"]]));
    }

    #[test]
    fn a_missing_trailing_pipe_still_splits_into_cells() {
        // E2E-150.
        let p = parse("| a | b\n| --- | ---\n| 1 | 2\n");
        assert_eq!(p.tables[0].cells, cells(&[&["a", "b"], &["1", "2"]]));
    }

    #[test]
    fn no_separator_no_leading_pipe_or_a_fence_is_no_table() {
        // E2E-146, E2E-148, E2E-145.
        for text in [
            "| a | b |\n| c | d |\n",
            "a | b\n--- | ---\n1 | 2\n",
            "```\n| a | b |\n| --- | --- |\n```\n",
            "| a |\n| -x- |\n",
            "| a | b |\n| --- | |\n",
        ] {
            assert_eq!(parse(text), Parsed::default(), "{text:?}");
        }
    }

    #[test]
    fn data_rows_stop_at_the_first_line_not_starting_with_a_pipe() {
        let p = parse("| a |\n| --- |\n| 1 |\ntext\n| 2 |\n");
        assert_eq!(p.tables[0].cells, cells(&[&["a"], &["1"]]));
        assert_eq!(p.tables[0].line_count, 3);
        let p = parse("  | a |\n  | --- |\n  | 1 |\n");
        assert_eq!(p.tables[0].cells, cells(&[&["a"], &["1"]]), "leading spaces are allowed");
    }

    #[test]
    fn a_picture_reference_is_a_line_holding_one_image_only() {
        let text = "![chart](figures/f1.png)\n  ![](b.jpg)  \nsee ![x](c.png)\n![a](d.png) ![b](e.png)\n```\n![in](code.png)\n```\n";
        let p = parse(text);
        assert_eq!(
            p.pictures,
            vec![
                PictureRef {
                    seq: 1,
                    alt: "chart".into(),
                    target: "figures/f1.png".into(),
                    line: 0
                },
                PictureRef { seq: 2, alt: String::new(), target: "b.jpg".into(), line: 1 },
            ]
        );
    }

    #[test]
    fn crlf_line_ends_are_read_as_line_ends() {
        let p = parse("| a |\r\n| --- |\r\n| 1 |\r\n");
        assert_eq!(p.tables[0].cells, cells(&[&["a"], &["1"]]));
    }
}
