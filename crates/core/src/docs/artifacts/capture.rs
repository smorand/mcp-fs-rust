//! Building one conversion's artifact set, weighing it for the session write
//! quota, reporting it, and committing it against the node revision it was
//! built from (SPEC-0019 DEC-005, DEC-008, FR-NEW-030).
//!
//! The caller charges [`PendingSet::bytes`] plus the sibling bytes ONCE, before
//! writing anything, then writes the sibling, then calls [`PendingSet::commit`].

use crate::docs::artifacts::parse::parse;
use crate::docs::artifacts::{CsvQuality, table_csv, table_markdown, width};
use crate::docs::extract::BuiltinTable;
use crate::errors::{Result, ToolError};
use crate::storage::VolumeClient;
use crate::storage::meta::{NewArtifactSet, NewDocTable, RelationalMetaStore};

/// One table about to be kept.
struct SetTable {
    seq: i64,
    caption: String,
    cells: Vec<Vec<String>>,
    quality: CsvQuality,
    line_start: usize,
    line_count: usize,
}

/// The set one conversion yields, not stored yet.
pub(crate) struct PendingSet<'a> {
    /// The conversion text.
    text: &'a str,
    tables: Vec<SetTable>,
    /// `<id>: <reason>` per failed item, images by id then tables by id
    /// (FR-NEW-015).
    failures: Vec<String>,
}

impl<'a> PendingSet<'a> {
    /// The built-in set of one text extraction (FR-NEW-017): its tables, exact,
    /// keeping only those whose whole Markdown lies before the cut when the text
    /// was cut (FR-NEW-034), numbered in reading order with every row.
    pub(crate) fn builtin(
        text: &'a str,
        tables: &[BuiltinTable],
        truncated_at: Option<usize>,
    ) -> Self {
        let tables = tables
            .iter()
            .filter(|t| truncated_at.is_none_or(|cut| t.char_end <= cut))
            .zip(1i64..)
            .map(|(t, seq)| SetTable {
                seq,
                caption: t.caption.clone(),
                cells: t.cells.clone(),
                quality: CsvQuality::Exact,
                line_start: t.line_start,
                line_count: t.line_count,
            })
            .collect();
        Self { text, tables, failures: Vec::new() }
    }

    /// The set of an external conversion (FR-NEW-033): its pipe tables,
    /// approximate, a malformed one reported under its id and not kept
    /// (FR-NEW-015). Pictures are kept from US-0005 on; until then a picture
    /// reference is neither kept nor reported.
    pub(crate) fn converter(text: &'a str) -> Self {
        let mut failures = Vec::new();
        let tables = parse(text)
            .tables
            .into_iter()
            .filter_map(|t| match t.cells {
                Some(cells) => Some(SetTable {
                    seq: t.seq,
                    caption: t.caption,
                    cells,
                    quality: CsvQuality::Approximate,
                    line_start: t.line_start,
                    line_count: t.line_count,
                }),
                None => {
                    failures.push(format!("table-{}: malformed table", t.seq));
                    None
                }
            })
            .collect();
        Self { text, tables, failures }
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

    /// The conversion report (FR-NEW-030): `<N> images, <M> tables`, then one
    /// line per failed item.
    pub(crate) fn report(&self) -> String {
        let head = format!("0 images, {} tables", self.tables.len());
        std::iter::once(head).chain(self.failures.iter().cloned()).collect::<Vec<_>>().join("\n")
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
            .map(|t| {
                Ok(NewDocTable {
                    seq: t.seq,
                    caption: t.caption.clone(),
                    rows: t.cells.len().saturating_sub(1) as i64,
                    cols: width(&t.cells) as i64,
                    quality: t.quality.as_str().to_string(),
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
            report: self.report(),
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
    fn a_cut_keeps_only_tables_ending_before_it() {
        let table = |end| BuiltinTable {
            caption: String::new(),
            cells: cells(&[&["h"]]),
            line_start: 0,
            line_count: 2,
            char_end: end,
        };
        let tables = vec![table(10), table(20)];
        assert_eq!(PendingSet::builtin("", &tables, None).tables.len(), 2);
        assert_eq!(PendingSet::builtin("", &tables, Some(20)).tables.len(), 2);
        assert_eq!(PendingSet::builtin("", &tables, Some(19)).tables.len(), 1);
        assert_eq!(PendingSet::builtin("", &tables, Some(9)).tables.len(), 0);
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
        assert_eq!(PendingSet::builtin("abcd", &tables, None).bytes(), 3 + 19 + 2 + 4);
    }

    #[test]
    fn a_converter_set_keeps_ids_and_reports_malformed_tables_in_order() {
        let text = "| a | b |\n| --- |\n\n| c |\n| --- |\n\n| d | e |\n| - |\n";
        let set = PendingSet::converter(text);
        assert_eq!(set.tables.iter().map(|t| t.seq).collect::<Vec<_>>(), vec![2]);
        assert_eq!(set.tables[0].quality, CsvQuality::Approximate);
        assert_eq!(
            set.report(),
            "0 images, 1 tables\ntable-1: malformed table\ntable-3: malformed table"
        );
        // The set is the kept table plus the text: `c` + `| c |\n| --- |`.
        assert_eq!(set.bytes(), (1 + 13 + text.len()) as i64);
    }
}
