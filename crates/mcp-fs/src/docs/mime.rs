//! MIME guessing by extension. Port of the C# `Mime.Guess` in `Core/Support.cs`
//! (itself a subset of Python's `mimetypes`). Used by `fs.read_bytes` to fill the
//! `mime` field and by the REST download route to set `Content-Type`.
//!
//! The table is intentionally a closed subset: a smaller, predictable answer set
//! is what the tool contract promises, so we do not pull a full MIME database.

/// Extension (lowercase, dot included) to MIME type. Kept sorted by family so a
/// human can diff it against the C# table at a glance.
const TABLE: &[(&str, &str)] = &[
    (".txt", "text/plain"),
    (".text", "text/plain"),
    (".log", "text/plain"),
    (".md", "text/markdown"),
    (".markdown", "text/markdown"),
    (".html", "text/html"),
    (".htm", "text/html"),
    (".css", "text/css"),
    (".csv", "text/csv"),
    (".json", "application/json"),
    (".xml", "application/xml"),
    (".js", "text/javascript"),
    (".mjs", "text/javascript"),
    (".yaml", "application/yaml"),
    (".yml", "application/yaml"),
    (".py", "text/x-python"),
    (".c", "text/x-csrc"),
    (".h", "text/x-chdr"),
    (".pdf", "application/pdf"),
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".bmp", "image/bmp"),
    (".webp", "image/webp"),
    (".tif", "image/tiff"),
    (".tiff", "image/tiff"),
    (".zip", "application/zip"),
    (".gz", "application/gzip"),
    (
        ".docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    (
        ".xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    (
        ".pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
];

/// Guess the MIME type from the path extension, or `None` when unknown.
///
/// Matches the C#: the extension is everything after the LAST dot of the whole
/// path (not of the file name), the lookup is case insensitive, and a path with
/// no dot at all yields `None`.
pub fn guess(path: &str) -> Option<&'static str> {
    let dot = path.rfind('.')?;
    let ext = path[dot..].to_ascii_lowercase();
    TABLE.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_the_whole_table() {
        for (ext, expected) in TABLE {
            let path = format!("/dir/file{ext}");
            assert_eq!(guess(&path), Some(*expected), "extension {ext}");
        }
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(guess("/A/README.MD"), Some("text/markdown"));
        assert_eq!(guess("/a/IMAGE.JPEG"), Some("image/jpeg"));
        assert_eq!(guess("/a/Report.PdF"), Some("application/pdf"));
    }

    #[test]
    fn unknown_extension_is_none() {
        assert_eq!(guess("/a/file.qqq"), None);
        assert_eq!(guess("/a/file.rs"), None, "rust source is not in the C# table");
    }

    #[test]
    fn no_dot_is_none() {
        assert_eq!(guess("/a/Makefile"), None);
        assert_eq!(guess(""), None);
    }

    #[test]
    fn office_types_are_the_long_ooxml_names() {
        assert_eq!(
            guess("/x.docx"),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        );
        assert_eq!(
            guess("/x.xlsx"),
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
        );
        assert_eq!(
            guess("/x.pptx"),
            Some("application/vnd.openxmlformats-officedocument.presentationml.presentation")
        );
    }

    #[test]
    fn last_dot_wins() {
        // matches the C# behaviour of scanning the full path for the last dot
        assert_eq!(guess("/a.pdf/b.txt"), Some("text/plain"));
        assert_eq!(guess("/archive.tar.gz"), Some("application/gzip"));
    }
}
