//! Document artifacts (SPEC-0019): the tables a conversion keeps beside the
//! Markdown sibling, their renderings and continuation markers. `parse` reads
//! the external converter's text, `capture` builds, weighs and commits a set.
//!
//! Artifacts are rows of the meta store plus content addressed blobs, never
//! `nodes` (DEC-002), so no enumerator of files ever sees them.

pub(crate) mod capture;
pub(crate) mod parse;
pub(crate) mod render;

pub(crate) use render::{table_csv, table_markdown, width};

use crate::errors::{Result, ToolError};
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
    /// Derived from the Markdown text of an external conversion.
    Approximate,
}

impl CsvQuality {
    /// The stored form.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Approximate => "approximate",
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
