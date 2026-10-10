//! Document artifacts (SPEC-0019): the tables a conversion keeps beside the
//! Markdown sibling, their continuation markers, and the capture of the set
//! the built-in conversion produces.
//!
//! Artifacts are rows of the meta store plus content addressed blobs, never
//! `nodes` (DEC-002), so no enumerator of files ever sees them.

use crate::docs::extract::BuiltinTable;
use crate::errors::{Result, ToolError};
use crate::storage::VolumeClient;
use crate::storage::meta::{NewArtifactSet, NewDocTable, RelationalMetaStore};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Items per page of an artifact list (FR-NEW-003, FR-NEW-005).
pub(crate) const ARTIFACT_PAGE_SIZE: usize = 100;
/// The refusal for a document never converted, or changed since (FR-NEW-023).
pub(crate) const NO_ARTIFACTS: &str = "No artifacts: document not extracted";
/// Marker format version, the first field of every marker (DEC-010).
const MARKER_VERSION: &str = "v1";

/// How a table's CSV was obtained (FR-NEW-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CsvQuality {
    /// Built from the original cells.
    Exact,
}

impl CsvQuality {
    /// The stored form.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
        }
    }

    /// The label a list reports, from the stored form.
    pub(crate) fn label(stored: &str) -> String {
        format!("CSV quality: {stored}")
    }
}

/// Which list a continuation marker belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListKind {
    Tables,
}

impl ListKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tables => "tables",
        }
    }
}

/// A continuation marker: base64url, no padding, of
/// `v1\n<project>\n<kind>\n<path>\n<offset>` (DEC-010). Stateless and bound
/// to the project, the list kind and the document, not to a person.
pub(crate) fn encode_marker(project: &str, kind: ListKind, path: &str, offset: usize) -> String {
    URL_SAFE_NO_PAD
        .encode(format!("{MARKER_VERSION}\n{project}\n{}\n{path}\n{offset}", kind.as_str()))
}

/// The offset a marker carries, when it was issued for this project, list kind
/// and document; anything else is `Invalid continuation marker: <raw>`.
pub(crate) fn decode_marker(raw: &str, project: &str, kind: ListKind, path: &str) -> Result<usize> {
    let invalid = || ToolError::invalid_argument(format!("Invalid continuation marker: {raw}"));
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let text = String::from_utf8(bytes).map_err(|_| invalid())?;
    // The path is the only field that may hold a newline, so the offset is
    // split from the right and the three fixed fields from the left.
    let (head, offset) = text.rsplit_once('\n').ok_or_else(invalid)?;
    let mut fields = head.splitn(4, '\n');
    let expected = [MARKER_VERSION, project, kind.as_str(), path];
    if expected.iter().any(|want| fields.next() != Some(*want)) {
        return Err(invalid());
    }
    offset.parse::<usize>().map_err(|_| invalid())
}

/// A cell as both renderings show it: a CR LF pair, a lone CR or a lone LF
/// becomes one space (FR-NEW-002).
fn flatten_newlines(cell: &str) -> String {
    cell.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// The widest row's cell count, header included (FR-NEW-002).
fn width(cells: &[Vec<String>]) -> usize {
    cells.iter().map(Vec::len).max().unwrap_or(0)
}

/// The Markdown rendering of a table (FR-NEW-002): header, separator, one line
/// per data row, short rows padded, `|` escaped, no trailing newline.
pub(crate) fn table_markdown(cells: &[Vec<String>]) -> String {
    let cols = width(cells);
    let line = |row: &[String]| {
        let parts: Vec<String> = (0..cols)
            .map(|i| {
                row.get(i).map_or_else(String::new, |c| flatten_newlines(c).replace('|', "\\|"))
            })
            .collect();
        format!("| {} |", parts.join(" | "))
    };
    let mut lines = Vec::with_capacity(cells.len() + 1);
    if let Some(header) = cells.first() {
        lines.push(line(header));
        lines.push(format!("| {} |", vec!["---"; cols].join(" | ")));
    }
    lines.extend(cells.iter().skip(1).map(|r| line(r)));
    lines.join("\n")
}

/// The CSV rendering of a table (FR-NEW-002): comma separated, quoted only
/// when needed, LF between lines, no trailing newline, short rows padded.
pub(crate) fn table_csv(cells: &[Vec<String>]) -> String {
    let cols = width(cells);
    let field = |c: &str| {
        if c.contains([',', '"', '\r', '\n']) {
            format!("\"{}\"", c.replace('"', "\"\""))
        } else {
            c.to_string()
        }
    };
    cells
        .iter()
        .map(|row| {
            (0..cols)
                .map(|i| row.get(i).map_or_else(String::new, |c| field(c)))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The set the built-in conversion yields for one text extraction
/// (FR-NEW-017): its tables, no images, and the conversion text.
pub(crate) struct BuiltinSet<'a> {
    text: &'a str,
    tables: Vec<&'a BuiltinTable>,
}

impl<'a> BuiltinSet<'a> {
    /// Keep only the tables whose whole Markdown lies before the cut, when the
    /// text was cut (FR-NEW-034); they keep reading order and every row.
    pub(crate) fn new(
        text: &'a str,
        tables: &'a [BuiltinTable],
        truncated_at: Option<usize>,
    ) -> Self {
        let tables =
            tables.iter().filter(|t| truncated_at.is_none_or(|cut| t.char_end <= cut)).collect();
        Self { text, tables }
    }

    /// What this set adds to the session write quota beside the sibling
    /// (FR-NEW-013): each table's CSV, Markdown and caption bytes, plus the
    /// conversion text.
    pub(crate) fn bytes(&self) -> i64 {
        let set: usize = self
            .tables
            .iter()
            .map(|t| table_csv(&t.cells).len() + table_markdown(&t.cells).len() + t.caption.len())
            .sum();
        (set + self.text.len()) as i64
    }

    /// Store the conversion text blob, then commit the set as the current one
    /// for `path`, refused when the node no longer carries `rev` (DEC-008).
    pub(crate) async fn commit(
        &self,
        client: &VolumeClient,
        store: &RelationalMetaStore,
        path: &str,
        rev: &str,
    ) -> Result<()> {
        let text_sha = if self.text.is_empty() {
            None
        } else {
            let sha = VolumeClient::sha256_hex(self.text.as_bytes());
            client.blob.put(&sha, self.text.as_bytes()).await?;
            Some(sha)
        };
        let tables = self
            .tables
            .iter()
            .zip(1i64..)
            .map(|(t, seq)| {
                Ok(NewDocTable {
                    seq,
                    caption: t.caption.clone(),
                    rows: t.cells.len().saturating_sub(1) as i64,
                    cols: width(&t.cells) as i64,
                    quality: CsvQuality::Exact.as_str().to_string(),
                    cells: serde_json::to_string(&t.cells)
                        .map_err(|e| ToolError::internal(format!("table cells: {e}")))?,
                    line_start: t.line_start as i64,
                    line_count: t.line_count as i64,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let set = NewArtifactSet {
            set_id: uuid::Uuid::new_v4().to_string(),
            path: path.to_string(),
            node_rev: rev.to_string(),
            text_sha,
            text_len: self.text.len() as i64,
            report: format!("0 images, {} tables", tables.len()),
            tables,
        };
        store.commit_artifact_set(&set).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|r| r.iter().map(|c| (*c).to_string()).collect()).collect()
    }

    #[test]
    fn marker_round_trips_and_is_scoped() {
        let m = encode_marker("proj", ListKind::Tables, "/a\nb.xlsx", 100);
        assert_eq!(decode_marker(&m, "proj", ListKind::Tables, "/a\nb.xlsx").unwrap(), 100);
        for (project, path) in [("other", "/a\nb.xlsx"), ("proj", "/c.xlsx")] {
            let err = decode_marker(&m, project, ListKind::Tables, path).unwrap_err();
            assert_eq!(err.message, format!("Invalid continuation marker: {m}"));
        }
        for raw in ["", "!!", "djE", &URL_SAFE_NO_PAD.encode("v1\nproj\ntables\n/x\nnope")] {
            assert!(decode_marker(raw, "proj", ListKind::Tables, "/x").is_err(), "{raw}");
        }
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
    fn a_cut_keeps_only_tables_ending_before_it() {
        let table = |end| BuiltinTable {
            caption: String::new(),
            cells: cells(&[&["h"]]),
            line_start: 0,
            line_count: 2,
            char_end: end,
        };
        let tables = vec![table(10), table(20)];
        assert_eq!(BuiltinSet::new("", &tables, None).tables.len(), 2);
        assert_eq!(BuiltinSet::new("", &tables, Some(20)).tables.len(), 2);
        assert_eq!(BuiltinSet::new("", &tables, Some(19)).tables.len(), 1);
        assert_eq!(BuiltinSet::new("", &tables, Some(9)).tables.len(), 0);
    }

    #[test]
    fn bytes_count_csv_markdown_caption_and_text() {
        let tables = vec![BuiltinTable {
            caption: "Q1".into(),
            cells: cells(&[&["h"], &["v"]]),
            line_start: 0,
            line_count: 3,
            char_end: 5,
        }];
        // csv "h\nv" (3) + markdown "| h |\n| --- |\n| v |" (19) + caption (2) + text (4)
        assert_eq!(BuiltinSet::new("abcd", &tables, None).bytes(), 3 + 19 + 2 + 4);
    }
}
