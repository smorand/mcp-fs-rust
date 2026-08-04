//! Text helpers matching the C# `Core/TextUtil.cs`: line splitting that preserves
//! the trailing-newline distinction, and glob matching for the fs.* tools.

/// Split into lines the way the C# implementation does: `\r\n`, `\r` and `\n` all
/// terminate a line, and a trailing terminator does NOT produce a final empty line.
pub fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                out.push(std::mem::take(&mut cur));
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// True when `text` ends with a line terminator.
pub fn ends_with_newline(text: &str) -> bool {
    text.ends_with('\n') || text.ends_with('\r')
}

/// Join lines back, appending a trailing newline when the original had one.
pub fn join_lines(lines: &[String], trailing_newline: bool) -> String {
    let mut s = lines.join("\n");
    if trailing_newline && !s.is_empty() {
        s.push('\n');
    }
    s
}

/// Count leading spaces/tabs (a tab counts as one character, like the C# version).
pub fn indent_width(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

/// Glob match on a whole path, supporting `*`, `?`, `**` and character classes.
/// `**` crosses '/' boundaries; a single `*` does not.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    match globset::GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map(|g| g.compile_matcher())
    {
        Ok(m) => m.is_match(path),
        Err(_) => false,
    }
}

/// Match a bare name (no '/') against a pattern, used for exclusion lists where a
/// pattern like `.git` or `*.tmp` should hit any path segment or file name.
pub fn name_match(pattern: &str, name: &str) -> bool {
    if pattern == name {
        return true;
    }
    match globset::Glob::new(pattern).map(|g| g.compile_matcher()) {
        Ok(m) => m.is_match(name),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_lines_no_trailing_empty() {
        assert_eq!(split_lines("a\nb\n"), vec!["a", "b"]);
        assert_eq!(split_lines("a\nb"), vec!["a", "b"]);
        assert_eq!(split_lines(""), Vec::<String>::new());
        assert_eq!(split_lines("\n"), vec![""]);
    }

    #[test]
    fn split_lines_handles_crlf_and_cr() {
        assert_eq!(split_lines("a\r\nb\r\n"), vec!["a", "b"]);
        assert_eq!(split_lines("a\rb"), vec!["a", "b"]);
        assert_eq!(split_lines("a\r\nb\nc\rd"), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn trailing_newline_detection() {
        assert!(ends_with_newline("a\n"));
        assert!(ends_with_newline("a\r\n"));
        assert!(!ends_with_newline("a"));
        assert!(!ends_with_newline(""));
    }

    #[test]
    fn join_round_trips() {
        let src = "a\nb\n";
        let lines = split_lines(src);
        assert_eq!(join_lines(&lines, ends_with_newline(src)), src);
        let src2 = "a\nb";
        let lines2 = split_lines(src2);
        assert_eq!(join_lines(&lines2, ends_with_newline(src2)), src2);
    }

    #[test]
    fn indent_width_counts_spaces_and_tabs() {
        assert_eq!(indent_width("    x"), 4);
        assert_eq!(indent_width("\t\tx"), 2);
        assert_eq!(indent_width("x"), 0);
    }

    #[test]
    fn glob_star_does_not_cross_slash() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "src/main.rs"));
        assert!(glob_match("src/*.rs", "src/main.rs"));
    }

    #[test]
    fn glob_double_star_matches_everything() {
        assert!(glob_match("**/*", "a/b/c.txt"));
        assert!(glob_match("**/*.py", "src/doc/x.py"));
    }

    #[test]
    fn name_match_exact_and_pattern() {
        assert!(name_match(".git", ".git"));
        assert!(name_match("*.tmp", "a.tmp"));
        assert!(!name_match("*.tmp", "a.txt"));
    }

    #[test]
    fn invalid_glob_does_not_panic() {
        assert!(!glob_match("[", "x"));
    }
}
