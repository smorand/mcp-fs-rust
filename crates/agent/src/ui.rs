//! Console presentation for the agent CLI: LLM markdown rendered as raw ANSI, plus the
//! tool call and tool result lines.
//!
//! The C# original went through a markup layer (Spectre.Console) before reaching the
//! terminal. Emitting ANSI ourselves removes that intermediate encoding, so there is a
//! single conversion step and no markup escaping hazard.

use std::fmt::Write as _;

use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const BOLD_WHITE: &str = "\x1b[1;97m";
const BOLD_UNDERLINE_WHITE: &str = "\x1b[1;4;97m";
const BOLD_YELLOW: &str = "\x1b[1;33m";

const GLYPH_GEAR: &str = "\u{2699}";
const GLYPH_OK: &str = "\u{2713}";
const GLYPH_ERR: &str = "\u{2717}";
const GLYPH_BULLET: &str = "\u{2022}";

/// Width of the rules that replace fences and thematic breaks. Fixed rather than
/// terminal derived so two runs of the same session produce identical transcripts.
const RULE_WIDTH: usize = 40;

/// Longest rendered argument value in a tool call summary.
const ARG_VALUE_COLS: usize = 60;
/// Longest tool result preview.
const PREVIEW_COLS: usize = 140;

/// Render LLM markdown output to stdout as ANSI. Blank input prints nothing.
pub fn render_markdown(text: &str) {
    if text.trim().is_empty() {
        return;
    }
    println!("{}", markdown_to_ansi(text));
}

/// Print the "tool is being invoked" line: a gear glyph, the tool name, and a dim
/// summary of the arguments when there is one.
pub fn tool_call(name: &str, args: &Value) {
    let summary = args_summary(args);
    // The blank line separates the call from whatever the model just streamed.
    println!();
    if summary.is_empty() {
        println!("  {YELLOW}{GLYPH_GEAR}{RESET} {BOLD_YELLOW}{name}{RESET}");
    } else {
        println!("  {YELLOW}{GLYPH_GEAR}{RESET} {BOLD_YELLOW}{name}{RESET}  {DIM}{summary}{RESET}");
    }
}

/// Print the tool result line, indented four spaces: a green check plus a one line
/// preview on success, a red cross plus the dim cause chain on failure.
pub fn tool_result(result: &str, is_error: bool) {
    if is_error {
        for line in error_lines(result) {
            println!("{line}");
        }
    } else {
        println!("    {GREEN}{GLYPH_OK} {}{RESET}", success_preview(result));
    }
}

/// Convert markdown to a string containing ANSI escapes. Pure, no I/O.
pub fn markdown_to_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;

    for raw in text.split('\n') {
        // Only the end is trimmed: leading spaces carry meaning in code and in nesting.
        let line = raw.trim_end();

        if line.starts_with("```") {
            // Both fences collapse to the same rule, so a block reads as a framed region.
            in_code = !in_code;
            let _ = writeln!(out, "{}", dim_rule());
        } else if in_code {
            let _ = writeln!(out, "{CYAN}{line}{RESET}");
        } else if line.starts_with('|') {
            // A table row is passed through: styling cells would break column alignment.
            let _ = writeln!(out, "{line}");
        } else if let Some(rest) = line.strip_prefix("### ") {
            let _ = writeln!(out, "{BOLD_WHITE}{rest}{RESET}");
        } else if let Some(rest) = line.strip_prefix("## ") {
            let _ = writeln!(out, "{BOLD_UNDERLINE_WHITE}{rest}{RESET}");
        } else if let Some(rest) = line.strip_prefix("# ") {
            let _ = writeln!(out, "{BOLD_UNDERLINE_WHITE}{rest}{RESET}");
        } else if let Some(rest) = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
        {
            let _ = writeln!(out, "  {DIM}{GLYPH_BULLET}{RESET} {}", inline_ansi(rest));
        } else if let Some(marker_len) = ordered_marker_len(line) {
            let (marker, rest) = line.split_at(marker_len);
            let _ = writeln!(out, "  {DIM}{marker}{RESET}{}", inline_ansi(rest));
        } else if line == "---" || line == "***" || line == "___" {
            let _ = writeln!(out, "{}", dim_rule());
        } else if line.is_empty() {
            out.push('\n');
        } else {
            let _ = writeln!(out, "{}", inline_ansi(line));
        }
    }

    out.trim_end().to_string()
}

/// A dim horizontal rule of exactly [`RULE_WIDTH`] box drawing characters.
fn dim_rule() -> String {
    format!("{DIM}{}{RESET}", "\u{2500}".repeat(RULE_WIDTH))
}

/// Length of an ordered list marker (digits, dot, space) at the start of `line`.
fn ordered_marker_len(line: &str) -> Option<usize> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    // Safe to slice: every counted byte is an ASCII digit, so this is a char boundary.
    if line[digits..].starts_with(". ") {
        Some(digits + 2)
    } else {
        None
    }
}

/// Apply inline emphasis. Scanning is left to right and the first rule that matches at a
/// position wins; content is emitted literally, so nesting is not resolved.
fn inline_ansi(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    // Two character markers are listed before their single character prefix so that
    // `**bold**` is never read as an empty italic run.
    let rules: [(&[char], &str); 6] = [
        (&['*', '*'], BOLD),
        (&['_', '_'], BOLD),
        (&['*'], ITALIC),
        (&['_'], ITALIC),
        (&['`'], CYAN),
        (&['~', '~'], DIM),
    ];

    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let mut styled = false;
        for (marker, style) in rules {
            let width = marker.len();
            if i + width > chars.len() || chars[i..i + width] != *marker {
                continue;
            }
            // The close must start past the first content character, so runs are non empty.
            if let Some(close) = find_marker(&chars, i + width + 1, marker) {
                let content: String = chars[i + width..close].iter().collect();
                let _ = write!(out, "{style}{content}{RESET}");
                i = close + width;
                styled = true;
            }
            // An opener without a close stays literal; no other rule is tried here.
            break;
        }
        if !styled {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Index of the first occurrence of `marker` in `chars` at or after `from`.
fn find_marker(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
    let width = marker.len();
    if width == 0 || chars.len() < width {
        return None;
    }
    (from..=chars.len() - width).find(|&j| chars[j..j + width] == *marker)
}

/// One line `key=value` summary of a tool call argument object. Null values and any non
/// object input yield nothing.
fn args_summary(args: &Value) -> String {
    let Some(map) = args.as_object() else {
        return String::new();
    };
    map.iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| {
            let rendered = render_arg(value);
            format!("{key}={}", truncate_cols(&rendered, ARG_VALUE_COLS, 57))
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// Compact one line form of a single argument value.
fn render_arg(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => format!("[{} items]", items.len()),
        Value::Object(_) => "{\u{2026}}".to_string(),
        Value::Null => String::new(),
    }
}

/// Collapse a successful tool result into a single capped line.
fn success_preview(result: &str) -> String {
    let segments: Vec<&str> = result
        .split('\n')
        .filter(|s| !s.is_empty())
        .map(str::trim)
        .collect();

    let preview = if segments.len() <= 3 {
        segments.join(" \u{b7} ")
    } else {
        format!(
            "{}  \u{2026} (+{} lines)",
            segments[..3].join(" \u{b7} "),
            segments.len() - 3
        )
    };

    let preview = truncate_cols(&preview, PREVIEW_COLS, 137);
    if preview.is_empty() {
        "(empty)".to_string()
    } else {
        preview
    }
}

/// The already indented lines of a failed tool result: headline then dim cause chain.
fn error_lines(result: &str) -> Vec<String> {
    result
        .split('\n')
        .filter(|s| !s.is_empty())
        .enumerate()
        .map(|(index, segment)| {
            if index == 0 {
                format!("    {RED}{GLYPH_ERR} {segment}{RESET}")
            } else {
                // The cause chain is context, so it sits behind the headline.
                format!("      {DIM}{}{RESET}", segment.trim())
            }
        })
        .collect()
}

/// Shorten `s` to `cut_to` display columns plus an ellipsis when it is wider than `max`.
/// Cuts on character boundaries, so multibyte input is never split.
fn truncate_cols(s: &str, max: usize, cut_to: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > cut_to {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule() -> String {
        format!("\x1b[2m{}\x1b[0m", "\u{2500}".repeat(40))
    }

    #[test]
    fn inline_bold_uses_both_markers() {
        assert_eq!(markdown_to_ansi("a **b** c"), "a \x1b[1mb\x1b[0m c");
        assert_eq!(markdown_to_ansi("a __b__ c"), "a \x1b[1mb\x1b[0m c");
    }

    #[test]
    fn inline_italic_uses_both_markers() {
        assert_eq!(markdown_to_ansi("a *b* c"), "a \x1b[3mb\x1b[0m c");
        assert_eq!(markdown_to_ansi("a _b_ c"), "a \x1b[3mb\x1b[0m c");
    }

    #[test]
    fn inline_code_is_cyan() {
        assert_eq!(markdown_to_ansi("run `ls` now"), "run \x1b[36mls\x1b[0m now");
    }

    #[test]
    fn inline_strikethrough_is_dim() {
        assert_eq!(markdown_to_ansi("a ~~b~~ c"), "a \x1b[2mb\x1b[0m c");
    }

    #[test]
    fn unclosed_markers_stay_literal() {
        assert_eq!(markdown_to_ansi("**bold"), "**bold");
        assert_eq!(markdown_to_ansi("`code"), "`code");
        assert_eq!(markdown_to_ansi("~~strike"), "~~strike");
        // An empty run has no closing marker past the first content character.
        assert_eq!(markdown_to_ansi("****"), "****");
    }

    #[test]
    fn headings_at_three_levels() {
        assert_eq!(markdown_to_ansi("# One"), "\x1b[1;4;97mOne\x1b[0m");
        assert_eq!(markdown_to_ansi("## Two"), "\x1b[1;4;97mTwo\x1b[0m");
        assert_eq!(markdown_to_ansi("### Three"), "\x1b[1;97mThree\x1b[0m");
    }

    #[test]
    fn fenced_block_becomes_rules_around_cyan_lines() {
        let got = markdown_to_ansi("```rust\nlet x = **1**;\n```");
        let want = format!("{}\n\x1b[36mlet x = **1**;\x1b[0m\n{}", rule(), rule());
        assert_eq!(got, want);
    }

    #[test]
    fn bullet_list_uses_dim_bullet() {
        assert_eq!(
            markdown_to_ansi("- one\n* **two**"),
            "  \x1b[2m\u{2022}\x1b[0m one\n  \x1b[2m\u{2022}\x1b[0m \x1b[1mtwo\x1b[0m"
        );
    }

    #[test]
    fn ordered_list_keeps_marker_dim() {
        assert_eq!(
            markdown_to_ansi("1. first\n12. `x`"),
            "  \x1b[2m1. \x1b[0mfirst\n  \x1b[2m12. \x1b[0m\x1b[36mx\x1b[0m"
        );
        // A digit run without the dot and space is ordinary text.
        assert_eq!(markdown_to_ansi("1.no space"), "1.no space");
    }

    #[test]
    fn horizontal_rules() {
        assert_eq!(markdown_to_ansi("---"), rule());
        assert_eq!(markdown_to_ansi("***"), rule());
        assert_eq!(markdown_to_ansi("___"), rule());
    }

    #[test]
    fn table_row_passes_through_untouched() {
        let row = "| a | **b** | `c` |";
        assert_eq!(markdown_to_ansi(row), row);
    }

    #[test]
    fn trailing_newlines_are_trimmed() {
        assert_eq!(markdown_to_ansi("hello\n\n\n"), "hello");
        assert_eq!(markdown_to_ansi("hello\nworld\n"), "hello\nworld");
        // Interior blank lines survive.
        assert_eq!(markdown_to_ansi("a\n\nb"), "a\n\nb");
    }

    #[test]
    fn blank_input_renders_to_nothing() {
        assert_eq!(markdown_to_ansi("   \n\t\n"), "");
    }

    #[test]
    fn args_summary_renders_every_json_kind_and_skips_nulls() {
        // Keys are alphabetical so the result holds whichever map ordering serde uses.
        let args = json!({
            "a_str": "proj",
            "b_num": 42,
            "c_bool": true,
            "d_arr": [1, 2, 3],
            "e_obj": {"k": "v"},
            "f_null": null,
        });
        assert_eq!(
            args_summary(&args),
            "a_str=proj  b_num=42  c_bool=true  d_arr=[3 items]  e_obj={\u{2026}}"
        );
    }

    #[test]
    fn args_summary_ignores_non_objects() {
        assert_eq!(args_summary(&Value::Null), "");
        assert_eq!(args_summary(&json!([1, 2])), "");
    }

    #[test]
    fn args_summary_truncates_a_long_value() {
        let args = json!({ "path": "x".repeat(80) });
        let want = format!("path={}\u{2026}", "x".repeat(57));
        assert_eq!(args_summary(&args), want);
    }

    #[test]
    fn truncate_cols_counts_display_width() {
        // Each ideograph is two columns wide, so four of them exceed a limit of four.
        assert_eq!(truncate_cols("\u{4f60}\u{597d}", 4, 2), "\u{4f60}\u{597d}");
        assert_eq!(
            truncate_cols("\u{4f60}\u{597d}\u{4e16}\u{754c}", 4, 2),
            "\u{4f60}\u{2026}"
        );
    }

    #[test]
    fn truncate_cols_never_splits_a_char() {
        // A wide emoji that does not fit is dropped whole rather than cut in half.
        let got = truncate_cols("\u{1f642}\u{1f642}\u{1f642}", 2, 2);
        assert_eq!(got, "\u{1f642}\u{2026}");
        let odd = truncate_cols("\u{1f642}\u{1f642}", 1, 1);
        assert_eq!(odd, "\u{2026}");
        assert!(odd.chars().all(|c| c == '\u{2026}'));
    }

    #[test]
    fn success_preview_joins_up_to_three_segments() {
        assert_eq!(
            success_preview("one\ntwo\nthree"),
            "one \u{b7} two \u{b7} three"
        );
    }

    #[test]
    fn success_preview_counts_the_extra_lines() {
        assert_eq!(
            success_preview("a\nb\nc\nd\ne"),
            "a \u{b7} b \u{b7} c  \u{2026} (+2 lines)"
        );
    }

    #[test]
    fn success_preview_caps_at_140_columns() {
        let got = success_preview(&"a".repeat(200));
        assert_eq!(got, format!("{}\u{2026}", "a".repeat(137)));
        assert_eq!(got.width(), 138);
    }

    #[test]
    fn success_preview_falls_back_to_empty_marker() {
        assert_eq!(success_preview(""), "(empty)");
        assert_eq!(success_preview("\n\n"), "(empty)");
    }

    #[test]
    fn error_lines_dim_the_cause_chain() {
        let got = error_lines("boom\n  caused by: io\n\n  and then: eof");
        assert_eq!(
            got,
            vec![
                "    \x1b[31m\u{2717} boom\x1b[0m".to_string(),
                "      \x1b[2mcaused by: io\x1b[0m".to_string(),
                "      \x1b[2mand then: eof\x1b[0m".to_string(),
            ]
        );
    }
}
