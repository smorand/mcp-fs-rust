//! Building one conversion's artifact set, weighing it for the session write
//! quota, reporting it, and committing it against the node revision it was
//! built from (SPEC-0019 DEC-005, DEC-008, FR-NEW-030).
//!
//! The caller charges [`PendingSet::bytes`] plus the sibling bytes ONCE, before
//! writing anything, then writes the sibling, then calls [`PendingSet::commit`].

use crate::docs::artifacts::parse::{PictureRef, parse};
use crate::docs::artifacts::{CsvQuality, table_csv, table_markdown, width};
use crate::docs::extract::BuiltinTable;
use crate::docs::service::Conversion;
use crate::errors::{Result, ToolError};
use crate::storage::VolumeClient;
use crate::storage::meta::{NewArtifactSet, NewDocImage, NewDocTable, RelationalMetaStore};

/// Source extensions whose pictures carry a page: the PDF page, the
/// PowerPoint slide (FR-NEW-001). Every other format has none.
const PAGED_SOURCES: [&str; 3] = ["pdf", "pptx", "ppt"];

/// The picture formats an image may have (FR-NEW-004).
const PICTURE_FORMATS: [&str; 6] = ["png", "jpeg", "gif", "webp", "tiff", "bmp"];

/// One image about to be kept, its bytes borrowed from the conversion.
struct SetImage<'a> {
    seq: i64,
    caption: String,
    page: Option<i64>,
    format: &'static str,
    bytes: &'a [u8],
    line: usize,
    /// Whether the converter marked its description as failed (FR-NEW-015):
    /// the image is kept with an empty caption but still reported.
    caption_failed: bool,
}

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
    images: Vec<SetImage<'a>>,
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
        Self { text, images: Vec::new(), tables, failures: Vec::new() }
    }

    /// The set of an external conversion of `source` (FR-NEW-033): its
    /// pictures, resolved only among the converter's own files (DEC-007), and
    /// its pipe tables, approximate, a malformed one reported under its id and
    /// not kept (FR-NEW-015). A picture that cannot be kept leaves its id
    /// unused and its failure reported (US-0006).
    pub(crate) fn converter(conversion: &'a Conversion, source: &str) -> Self {
        let text = conversion.markdown.as_str();
        let parsed = parse(text);
        let paged = extension(source).is_some_and(|e| PAGED_SOURCES.contains(&e.as_str()));
        let mut failures = Vec::new();
        let mut images = Vec::with_capacity(parsed.pictures.len());
        for r in &parsed.pictures {
            match picture(conversion, r, paged) {
                Ok(image) => {
                    if image.caption_failed {
                        failures.push(format!("image-{}: caption unavailable", image.seq));
                    }
                    images.push(image);
                }
                Err(reason) => failures.push(format!("image-{}: {reason}", r.seq)),
            }
        }
        let tables = parsed
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
        Self { text, images, tables, failures }
    }

    /// What this set adds to the session write quota beside the sibling
    /// (FR-NEW-013): each image's picture and caption bytes, each table's
    /// CSV, Markdown and caption bytes, plus the conversion text. A picture
    /// placed twice is charged twice: each placement is its own image.
    pub(crate) fn bytes(&self) -> i64 {
        let images: usize = self.images.iter().map(|i| i.bytes.len() + i.caption.len()).sum();
        let tables: usize = self
            .tables
            .iter()
            .map(|t| table_csv(&t.cells).len() + table_markdown(&t.cells).len() + t.caption.len())
            .sum();
        (images + tables + self.text.len()) as i64
    }

    /// The conversion report (FR-NEW-030): `<N> images, <M> tables`, then one
    /// line per failed item.
    pub(crate) fn report(&self) -> String {
        let head = format!("{} images, {} tables", self.images.len(), self.tables.len());
        std::iter::once(head).chain(self.failures.iter().cloned()).collect::<Vec<_>>().join("\n")
    }

    /// Store the conversion text and picture blobs, then commit the set as the
    /// current one for `path`, refused when the node no longer carries `rev`
    /// (DEC-008). A refused commit leaves the blobs unreferenced, as an
    /// interrupted write does.
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
        let mut images = Vec::with_capacity(self.images.len());
        for i in &self.images {
            let sha = VolumeClient::sha256_hex(i.bytes);
            client.blob.put(&sha, i.bytes).await?;
            images.push(NewDocImage {
                seq: i.seq,
                caption: i.caption.clone(),
                page: i.page,
                format: i.format.to_string(),
                sha256: sha,
                size: i.bytes.len() as i64,
                line_start: i.line as i64,
                line_count: 1,
            });
        }
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
            images,
            tables,
        };
        store.commit_artifact_set(&set).await
    }
}

/// The image a picture reference yields, or the FR-NEW-015 reason it cannot
/// be kept: its target resolves to nothing inside the converter's output, no
/// file of the conversion carries that key, or its format is not a picture
/// format.
fn picture<'a>(
    conversion: &'a Conversion,
    r: &PictureRef,
    paged: bool,
) -> std::result::Result<SetImage<'a>, String> {
    let unavailable = || "picture unavailable".to_string();
    let key = resolve_target(&r.target).ok_or_else(unavailable)?;
    let bytes = conversion.files.get(&key).ok_or_else(unavailable)?;
    let ext = extension(&key).unwrap_or_default();
    let format = picture_format(&key).ok_or_else(|| format!("unsupported picture format {ext}"))?;
    // The manifest is keyed by the target as written; a converter keying it
    // by the resolved file path is read the same way.
    let meta = conversion
        .manifest
        .as_ref()
        .and_then(|m| m.pictures.get(&r.target).or_else(|| m.pictures.get(&key)));
    let caption_failed = meta.is_some_and(|m| m.caption_failed);
    // A failed description leaves the caption empty (FR-NEW-033).
    let caption = meta.filter(|m| !m.caption_failed).map(|m| m.caption.clone()).unwrap_or_default();
    let page = if paged { meta.and_then(|m| m.page) } else { None };
    Ok(SetImage { seq: r.seq, caption, page, format, bytes, line: r.line, caption_failed })
}

/// The `Conversion.files` key a target names (DEC-007): its relative path
/// with `.` and empty segments dropped. A URL, an absolute path or a `..`
/// leaving the converter's output names nothing, so nothing outside it is
/// ever read.
fn resolve_target(target: &str) -> Option<String> {
    if target.contains(':') || target.starts_with('/') || target.starts_with('\\') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The lowercase extension of a path's last segment.
fn extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let (_, ext) = name.rsplit_once('.')?;
    Some(ext.to_ascii_lowercase())
}

/// The FR-NEW-004 format of a picture file: `jpeg` for `.jpg`/`.jpeg`, `tiff`
/// for `.tif`/`.tiff`, otherwise its lowercase extension when it is listed.
fn picture_format(path: &str) -> Option<&'static str> {
    let ext = extension(path)?;
    let ext = match ext.as_str() {
        "jpg" => "jpeg",
        "tif" => "tiff",
        other => other,
    };
    PICTURE_FORMATS.iter().copied().find(|f| *f == ext)
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
        let conversion = Conversion::text(text);
        let set = PendingSet::converter(&conversion, "/t.pdf");
        assert_eq!(set.tables.iter().map(|t| t.seq).collect::<Vec<_>>(), vec![2]);
        assert_eq!(set.tables[0].quality, CsvQuality::Approximate);
        assert_eq!(
            set.report(),
            "0 images, 1 tables\ntable-1: malformed table\ntable-3: malformed table"
        );
        // The set is the kept table plus the text: `c` + `| c |\n| --- |`.
        assert_eq!(set.bytes(), (1 + 13 + text.len()) as i64);
    }

    fn pictures(
        md: &str,
        files: &[(&str, &[u8])],
        source: &str,
    ) -> Vec<(i64, String, Option<i64>)> {
        use crate::docs::service::{Manifest, PictureMeta};
        let conversion = Conversion {
            markdown: md.to_string(),
            files: files.iter().map(|(k, v)| ((*k).to_string(), v.to_vec())).collect(),
            manifest: Some(Manifest {
                pictures: files
                    .iter()
                    .map(|(k, _)| {
                        let meta = PictureMeta {
                            caption: format!("c {k}"),
                            caption_failed: false,
                            page: Some(4),
                        };
                        ((*k).to_string(), meta)
                    })
                    .collect(),
            }),
        };
        let set = PendingSet::converter(&conversion, source);
        set.images.iter().map(|i| (i.seq, i.format.to_string(), i.page)).collect()
    }

    #[test]
    fn targets_resolve_only_inside_the_converter_output() {
        assert_eq!(resolve_target("figures/f1.png").as_deref(), Some("figures/f1.png"));
        assert_eq!(resolve_target("./figures//f1.png").as_deref(), Some("figures/f1.png"));
        assert_eq!(resolve_target("a/../f1.png").as_deref(), Some("f1.png"));
        for escape in [
            "../secret.png",
            "a/../../secret.png",
            "/etc/x.png",
            "https://h/x.png",
            "data:image/png;base64,AA",
            "C:\\x.png",
            "\\\\host\\x.png",
            "",
            ".",
        ] {
            assert_eq!(resolve_target(escape), None, "{escape}");
        }
    }

    #[test]
    fn picture_formats_follow_the_extension() {
        let cases = [
            ("a.PNG", Some("png")),
            ("a.jpg", Some("jpeg")),
            ("a.JPEG", Some("jpeg")),
            ("a.tif", Some("tiff")),
            ("a.tiff", Some("tiff")),
            ("a.gif", Some("gif")),
            ("a.webp", Some("webp")),
            ("a.bmp", Some("bmp")),
            ("a.svg", None),
            ("noext", None),
            ("dir.png/noext", None),
        ];
        for (path, want) in cases {
            assert_eq!(picture_format(path), want, "{path}");
        }
    }

    #[test]
    fn a_picture_that_cannot_be_kept_leaves_its_id_unused_without_failing() {
        let md = "![](a.png)\n![](missing.png)\n![](b.svg)\n![](../a.png)\n![](c.jpg)\n";
        let got = pictures(md, &[("a.png", b"a"), ("b.svg", b"b"), ("c.jpg", b"c")], "/r.pdf");
        assert_eq!(got, vec![(1, "png".into(), Some(4)), (5, "jpeg".into(), Some(4))]);
    }

    #[test]
    fn only_pdf_and_powerpoint_pictures_carry_a_page() {
        let files: &[(&str, &[u8])] = &[("a.png", b"a")];
        for (source, page) in [
            ("/r.pdf", Some(4)),
            ("/d.PPTX", Some(4)),
            ("/m.docx", None),
            ("/p.html", None),
            ("/x", None),
        ] {
            assert_eq!(pictures("![](a.png)\n", files, source)[0].2, page, "{source}");
        }
    }

    #[test]
    fn a_failed_or_absent_description_is_an_empty_caption_and_bytes_count_pictures() {
        use crate::docs::service::{Manifest, PictureMeta};
        let failed = PictureMeta { caption: "ignored".into(), caption_failed: true, page: None };
        let conversion = Conversion {
            markdown: "![](a.png)\n![](b.png)\n".into(),
            files: [("a.png".to_string(), b"aaaa".to_vec()), ("b.png".to_string(), b"bb".to_vec())]
                .into(),
            manifest: Some(Manifest { pictures: [("a.png".to_string(), failed)].into() }),
        };
        let set = PendingSet::converter(&conversion, "/r.pdf");
        assert!(set.images.iter().all(|i| i.caption.is_empty()));
        assert_eq!(set.report(), "2 images, 0 tables\nimage-1: caption unavailable");
        assert_eq!(set.bytes(), (4 + 2 + conversion.markdown.len()) as i64);
    }
}
