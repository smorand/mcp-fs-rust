//! Unified diff over lines, a port of the C# `Core/Diff.cs` (itself modelled on
//! Python `difflib.unified_diff` with context = 3).
//!
//! The opcode grouping is delegated to `similar::group_diff_ops`, which implements
//! exactly the same trimming rules as difflib's `get_grouped_opcodes`, so hunk
//! boundaries land in the same places as the C# implementation.

use similar::{Algorithm, DiffTag, capture_diff_slices, group_diff_ops};

/// Number of unchanged lines kept around each hunk, matching the C# default.
pub const DEFAULT_CONTEXT: usize = 3;

/// Split into lines keeping the terminators, the Rust twin of the C#
/// `TextUtil.SplitLinesKeepEnds`. Needed because the diff, the fuzzy block
/// replace and `insert_at_line` all reassemble text by concatenating lines,
/// which only round-trips when the terminators are preserved.
pub fn split_lines_keep_ends(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                // '\r\n' is one terminator, a lone '\r' is also one.
                let mut end = i + 1;
                if end < bytes.len() && bytes[end] == b'\n' {
                    end += 1;
                }
                out.push(&text[start..end]);
                i = end;
                start = end;
            }
            b'\n' => {
                out.push(&text[start..i + 1]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        out.push(&text[start..]);
    }
    out
}

/// Unified diff with the default context, labelled with `path` on both sides
/// (the C# implementation uses the same path for `---` and `+++`).
pub fn unified(old_text: &str, new_text: &str, path: &str) -> String {
    unified_with_context(old_text, new_text, path, DEFAULT_CONTEXT)
}

/// Unified diff with an explicit context size. Returns an empty string when the
/// two texts are identical, exactly like the C# version.
pub fn unified_with_context(
    old_text: &str,
    new_text: &str,
    path: &str,
    context: usize,
) -> String {
    let a = split_lines_keep_ends(old_text);
    let b = split_lines_keep_ends(new_text);
    let ops = capture_diff_slices(Algorithm::Myers, &a, &b);
    if ops.iter().all(|o| o.tag() == DiffTag::Equal) {
        return String::new();
    }
    let groups = group_diff_ops(ops, context);
    if groups.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("--- ");
    out.push_str(path);
    out.push('\n');
    out.push_str("+++ ");
    out.push_str(path);
    out.push('\n');

    for group in &groups {
        let first = group[0];
        let last = group[group.len() - 1];
        let (a1, a2) = (first.old_range().start, last.old_range().end);
        let (b1, b2) = (first.new_range().start, last.new_range().end);
        out.push_str("@@ -");
        out.push_str(&format_range(a1, a2));
        out.push_str(" +");
        out.push_str(&format_range(b1, b2));
        out.push_str(" @@\n");
        for op in group {
            if op.tag() == DiffTag::Equal {
                for line in &a[op.old_range()] {
                    push_line(&mut out, ' ', line);
                }
            } else {
                // Deletions first, then insertions, matching difflib and the C# port.
                for line in &a[op.old_range()] {
                    push_line(&mut out, '-', line);
                }
                for line in &b[op.new_range()] {
                    push_line(&mut out, '+', line);
                }
            }
        }
    }
    out
}

/// A line that carries no terminator (the last line of a file) still gets one in
/// the diff body, so hunks stay line oriented. Same rule as the C# `AppendLine`.
fn push_line(out: &mut String, prefix: char, line: &str) {
    out.push(prefix);
    out.push_str(line);
    if !line.ends_with('\n') {
        out.push('\n');
    }
}

/// difflib's `_format_range_unified`: a zero length range keeps the raw start.
fn format_range(start: usize, stop: usize) -> String {
    let length = stop - start;
    let begin = if length == 0 { start } else { start + 1 };
    if length == 1 {
        begin.to_string()
    } else {
        format!("{begin},{length}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_ends_preserves_terminators() {
        assert_eq!(split_lines_keep_ends("a\nb\n"), vec!["a\n", "b\n"]);
        assert_eq!(split_lines_keep_ends("a\nb"), vec!["a\n", "b"]);
        assert_eq!(split_lines_keep_ends(""), Vec::<&str>::new());
        assert_eq!(split_lines_keep_ends("\n"), vec!["\n"]);
        assert_eq!(split_lines_keep_ends("a\r\nb\rc"), vec!["a\r\n", "b\r", "c"]);
    }

    #[test]
    fn keep_ends_round_trips_unicode() {
        let src = "héllo\nwörld ✅\n";
        assert_eq!(split_lines_keep_ends(src).concat(), src);
    }

    #[test]
    fn identical_text_has_no_diff() {
        assert_eq!(unified("a\nb\n", "a\nb\n", "/f.txt"), "");
        assert_eq!(unified("", "", "/f.txt"), "");
    }

    #[test]
    fn single_line_change_produces_one_hunk() {
        let d = unified("a\nb\nc\n", "a\nB\nc\n", "/f.txt");
        assert_eq!(
            d,
            "--- /f.txt\n+++ /f.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"
        );
    }

    #[test]
    fn insertion_into_empty_file() {
        let d = unified("", "x\n", "/f.txt");
        assert_eq!(d, "--- /f.txt\n+++ /f.txt\n@@ -0,0 +1 @@\n+x\n");
    }

    #[test]
    fn deletion_to_empty_file() {
        let d = unified("x\n", "", "/f.txt");
        assert_eq!(d, "--- /f.txt\n+++ /f.txt\n@@ -1 +0,0 @@\n-x\n");
    }

    #[test]
    fn missing_final_newline_still_gets_one_in_the_body() {
        let d = unified("a", "b", "/f.txt");
        assert_eq!(d, "--- /f.txt\n+++ /f.txt\n@@ -1 +1 @@\n-a\n+b\n");
    }

    #[test]
    fn distant_changes_produce_two_hunks() {
        let old: String = (1..=20).map(|i| format!("line{i}\n")).collect();
        let mut lines: Vec<String> = (1..=20).map(|i| format!("line{i}\n")).collect();
        lines[0] = "LINE1\n".into();
        lines[19] = "LINE20\n".into();
        let new: String = lines.concat();
        let d = unified(&old, &new, "/f.txt");
        assert_eq!(d.matches("@@ -").count(), 2, "two separate hunks: {d}");
        assert!(d.contains("-line1\n+LINE1\n"));
        assert!(d.contains("-line20\n+LINE20\n"));
        // context is capped at three lines on each side
        assert!(d.contains(" line4\n"));
        assert!(!d.contains(" line5\n"));
    }

    #[test]
    fn nearby_changes_stay_in_one_hunk() {
        let d = unified("a\nb\nc\nd\ne\n", "A\nb\nc\nd\nE\n", "/f.txt");
        assert_eq!(d.matches("@@ -").count(), 1, "one hunk: {d}");
    }

    #[test]
    fn context_size_is_configurable() {
        let old: String = (1..=11).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l6\n", "L6\n");
        let d = unified_with_context(&old, &new, "/f.txt", 1);
        assert_eq!(d, "--- /f.txt\n+++ /f.txt\n@@ -5,3 +5,3 @@\n l5\n-l6\n+L6\n l7\n");
    }

    #[test]
    fn header_uses_the_same_path_twice() {
        let d = unified("a\n", "b\n", "/some/deep/file.rs");
        assert!(d.starts_with("--- /some/deep/file.rs\n+++ /some/deep/file.rs\n"));
    }

    #[test]
    fn pure_append_reports_insert_only() {
        let d = unified("a\n", "a\nb\n", "/f.txt");
        assert_eq!(d, "--- /f.txt\n+++ /f.txt\n@@ -1 +1,2 @@\n a\n+b\n");
    }
}
