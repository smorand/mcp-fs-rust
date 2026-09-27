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

/// Python `fnmatch` semantics, translated to a regex exactly like the C#
/// `TextUtil.FnmatchToRegex`. This is the ONLY glob implementation in the tree, on
/// purpose: `fs.glob` and `fs.grep` parity depends on a single `*` crossing '/'
/// boundaries, which `globset` with `literal_separator(true)` does NOT do. Verified
/// against the reference server: pattern `*.rs` matches `/src/nested.rs`.
pub struct Fnmatch(Option<regex::Regex>);

impl Fnmatch {
    pub fn new(pattern: &str) -> Self {
        let chars: Vec<char> = pattern.chars().collect();
        let mut source = String::from("(?s)^");
        let mut i = 0usize;
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            match c {
                '*' => source.push_str(".*"),
                '?' => source.push('.'),
                '[' => {
                    let mut j = i;
                    if j < chars.len() && (chars[j] == '!' || chars[j] == '^') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j] == ']' {
                        j += 1;
                    }
                    while j < chars.len() && chars[j] != ']' {
                        j += 1;
                    }
                    if j >= chars.len() {
                        // Unterminated class: a literal '[', like fnmatch.
                        source.push_str("\\[");
                    } else {
                        let inner: String = chars[i..j].iter().collect();
                        i = j + 1;
                        let mut inner = inner.replace('\\', "\\\\");
                        if let Some(rest) = inner.strip_prefix('!') {
                            inner = format!("^{rest}");
                        }
                        source.push('[');
                        source.push_str(&inner);
                        source.push(']');
                    }
                }
                other => source.push_str(&regex::escape(&other.to_string())),
            }
        }
        source.push('$');
        // An unrepresentable class matches nothing rather than blowing up the call.
        Self(regex::Regex::new(&source).ok())
    }

    pub fn is_match(&self, name: &str) -> bool {
        self.0.as_ref().is_some_and(|r| r.is_match(name))
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

    /// The behaviour that matters for parity: a single `*` crosses '/' like Python
    /// fnmatch, verified against the reference server (`*.rs` matches `/src/nested.rs`).
    #[test]
    fn star_crosses_slash_like_python_fnmatch() {
        assert!(Fnmatch::new("*.rs").is_match("main.rs"));
        assert!(Fnmatch::new("*.rs").is_match("/src/nested.rs"));
        assert!(Fnmatch::new("**/*.rs").is_match("/src/nested.rs"));
        assert!(Fnmatch::new("src/*.rs").is_match("src/main.rs"));
    }

    #[test]
    fn question_mark_matches_one_char() {
        assert!(Fnmatch::new("a?c").is_match("abc"));
        assert!(!Fnmatch::new("a?c").is_match("ac"));
    }

    #[test]
    fn character_classes_and_negation() {
        assert!(Fnmatch::new("[ab]x").is_match("ax"));
        assert!(!Fnmatch::new("[ab]x").is_match("cx"));
        assert!(Fnmatch::new("[!ab]x").is_match("cx"));
        assert!(!Fnmatch::new("[!ab]x").is_match("ax"));
    }

    #[test]
    fn literal_dot_is_escaped() {
        assert!(Fnmatch::new("a.txt").is_match("a.txt"));
        assert!(!Fnmatch::new("a.txt").is_match("axtxt"));
    }

    #[test]
    fn an_unterminated_class_is_a_literal_bracket() {
        assert!(Fnmatch::new("[").is_match("["));
        assert!(!Fnmatch::new("[").is_match("x"));
    }
}
