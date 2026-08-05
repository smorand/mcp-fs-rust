//! A readline style line editor: arrows, history, Ctrl+A/E/K/U, multi line continuation.
//!
//! Terminal wrap aware. The invariant every redraw depends on:
//!
//! - `from` is the content cursor position where the PHYSICAL terminal cursor currently
//!   sits, which is wherever the last draw left it.
//! - `to` is where the cursor must end up.
//!
//! Tracking `from` separately is what makes editing in the middle of a wrapped line
//! correct: computing the row from the buffer length instead would move up the wrong
//! number of rows as soon as the cursor is not at the end.
//!
//! Columns are display widths, not character counts, so a CJK or emoji character that
//! occupies two cells does not desynchronise the wrap arithmetic.

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthChar;

const HISTORY_CAP: usize = 1000;

/// What one read produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Input {
    /// A finished logical line.
    Line(String),
    /// Ctrl+C: abandon the current line but keep the session.
    Interrupted,
    /// Ctrl+D on an empty line: quit.
    Eof,
}

/// The line editor, owning the persistent history file.
pub struct InputReader {
    history: Vec<String>,
    history_file: PathBuf,
}

impl InputReader {
    /// Load history from `history_file` if it exists, keeping the last entries only.
    pub fn new(history_file: &Path) -> Self {
        let mut history: Vec<String> = std::fs::read_to_string(history_file)
            .map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect())
            .unwrap_or_default();
        if history.len() > HISTORY_CAP {
            history.drain(..history.len() - HISTORY_CAP);
        }
        Self { history, history_file: history_file.to_path_buf() }
    }

    /// Remember a line, skipping blanks and an immediate repeat, and persist it.
    pub fn add_history(&mut self, line: &str) {
        if line.trim().is_empty() || self.history.last().is_some_and(|l| l == line) {
            return;
        }
        self.history.push(line.to_string());
        if let Some(parent) = self.history_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) =
            std::fs::OpenOptions::new().create(true).append(true).open(&self.history_file)
        {
            let _ = writeln!(f, "{line}");
        }
    }

    /// Trim the file back to the cap on exit, so it cannot grow without bound.
    pub fn save(&self) {
        if self.history.len() > HISTORY_CAP {
            let tail = &self.history[self.history.len() - HISTORY_CAP..];
            let _ = std::fs::write(&self.history_file, tail.join("\n") + "\n");
        }
    }

    /// Read one logical line, joining continuations. A line ending in a backslash
    /// continues on the next one, with the backslash removed: terminals cannot report
    /// Shift+Enter, so this is the portable way to type a multi line prompt.
    pub fn read_input(&mut self, prompt: &str, continuation: &str) -> Result<Input> {
        let mut lines: Vec<String> = Vec::new();
        loop {
            let p = if lines.is_empty() { prompt } else { continuation };
            print!("{p}");
            std::io::stdout().flush()?;

            match self.read_line(p)? {
                Input::Line(line) => {
                    if let Some(head) = line.strip_suffix('\\') {
                        lines.push(head.to_string());
                        continue;
                    }
                    lines.push(line);
                    break;
                }
                other => return Ok(other),
            }
        }

        let joined = lines.join("\n");
        // Only the first physical line goes into history: recalling a wall of text is
        // not useful, and it keeps one history entry per prompt.
        if let Some(first) = joined.split('\n').next() {
            self.add_history(first);
        }
        Ok(Input::Line(joined))
    }

    /// The core editing loop for one physical line.
    ///
    /// Without a terminal there is nothing to edit and raw mode would fail, so a piped or
    /// redirected stdin falls back to a plain read. That is what makes the agent
    /// scriptable: `echo "list my projects" | ./agent.sh --user me` works.
    fn read_line(&mut self, prompt: &str) -> Result<Input> {
        if !std::io::stdin().is_terminal() {
            let mut line = String::new();
            let n = std::io::stdin().lock().read_line(&mut line)?;
            if n == 0 {
                return Ok(Input::Eof);
            }
            let line = line.trim_end_matches(['\n', '\r']).to_string();
            // Echo it so a captured transcript shows the prompt and the request together.
            println!("{line}");
            return Ok(Input::Line(line));
        }
        self.read_line_interactive(prompt)
    }

    /// The interactive editor, only reached when stdin is a terminal.
    fn read_line_interactive(&mut self, prompt: &str) -> Result<Input> {
        // Only the part of the prompt after the last newline shares the cursor's row, and
        // its colour escapes occupy no cells.
        let prompt_cols = visible_width(visible_prompt(prompt));

        let mut buf: Vec<char> = Vec::new();
        let mut cursor = 0usize;
        let mut hist_idx = self.history.len();
        let mut saved = String::new();

        terminal::enable_raw_mode()?;
        let outcome = loop {
            let Event::Key(KeyEvent { code, modifiers, kind, .. }) = crossterm::event::read()?
            else {
                continue;
            };
            // Windows and some terminals report press and release; act on press only.
            if kind != crossterm::event::KeyEventKind::Press {
                continue;
            }
            let ctrl = modifiers.contains(KeyModifiers::CONTROL);

            match code {
                KeyCode::Char('d') if ctrl => {
                    if buf.is_empty() {
                        break Input::Eof;
                    }
                }
                KeyCode::Char('c') if ctrl => {
                    // Leave the partial line visible, then hand back control.
                    move_to_end(prompt_cols, &buf, cursor);
                    print!("^C\r\n");
                    break Input::Interrupted;
                }
                KeyCode::Enter => {
                    move_to_end(prompt_cols, &buf, cursor);
                    print!("\r\n");
                    break Input::Line(buf.iter().collect());
                }
                KeyCode::Backspace => {
                    if cursor > 0 {
                        // Measure against the buffer as it still is: after the removal the
                        // widths shift, and the physical cursor has not moved yet.
                        let from_row = row_of(prompt_cols, &buf, cursor);
                        buf.remove(cursor - 1);
                        cursor -= 1;
                        redraw(prompt, prompt_cols, &buf, from_row, cursor);
                    }
                }
                KeyCode::Delete => {
                    if cursor < buf.len() {
                        let from_row = row_of(prompt_cols, &buf, cursor);
                        buf.remove(cursor);
                        redraw(prompt, prompt_cols, &buf, from_row, cursor);
                    }
                }
                KeyCode::Left => {
                    if cursor > 0 {
                        let from = cursor;
                        cursor -= 1;
                        move_cursor(prompt_cols, &buf, from, cursor);
                    }
                }
                KeyCode::Right => {
                    if cursor < buf.len() {
                        let from = cursor;
                        cursor += 1;
                        move_cursor(prompt_cols, &buf, from, cursor);
                    }
                }
                KeyCode::Home | KeyCode::Char('a') if ctrl || code == KeyCode::Home => {
                    let from = cursor;
                    cursor = 0;
                    move_cursor(prompt_cols, &buf, from, cursor);
                }
                KeyCode::End | KeyCode::Char('e') if ctrl || code == KeyCode::End => {
                    let from = cursor;
                    cursor = buf.len();
                    move_cursor(prompt_cols, &buf, from, cursor);
                }
                KeyCode::Char('k') if ctrl => {
                    // The physical cursor sits at `cursor`, not at the old end: the text
                    // after it is erased by the redraw either way.
                    let from_row = row_of(prompt_cols, &buf, cursor);
                    buf.truncate(cursor);
                    redraw(prompt, prompt_cols, &buf, from_row, cursor);
                }
                KeyCode::Char('u') if ctrl => {
                    let from_row = row_of(prompt_cols, &buf, cursor);
                    buf.drain(..cursor);
                    cursor = 0;
                    redraw(prompt, prompt_cols, &buf, from_row, cursor);
                }
                KeyCode::Up => {
                    if hist_idx > 0 {
                        if hist_idx == self.history.len() {
                            saved = buf.iter().collect();
                        }
                        hist_idx -= 1;
                        let entry = self.history[hist_idx].clone();
                        let from_row = row_of(prompt_cols, &buf, cursor);
                        replace_buffer(prompt, prompt_cols, &mut buf, &mut cursor, &entry, from_row);
                    }
                }
                KeyCode::Down => {
                    if hist_idx < self.history.len() {
                        hist_idx += 1;
                        let entry = if hist_idx == self.history.len() {
                            saved.clone()
                        } else {
                            self.history[hist_idx].clone()
                        };
                        let from_row = row_of(prompt_cols, &buf, cursor);
                        replace_buffer(prompt, prompt_cols, &mut buf, &mut cursor, &entry, from_row);
                    }
                }
                KeyCode::Char(c) if !ctrl && !modifiers.contains(KeyModifiers::ALT) => {
                    let from_row = row_of(prompt_cols, &buf, cursor);
                    buf.insert(cursor, c);
                    cursor += 1;
                    redraw(prompt, prompt_cols, &buf, from_row, cursor);
                }
                _ => {}
            }
        };
        terminal::disable_raw_mode()?;
        let _ = std::io::stdout().flush();
        Ok(outcome)
    }
}

// ── terminal geometry ────────────────────────────────────────────────────────────

/// Usable columns, floored so the arithmetic never divides by a silly width.
fn term_cols() -> usize {
    terminal::size().map(|(c, _)| c as usize).unwrap_or(80).max(20)
}

/// The part of a prompt that shares the cursor's row: everything after the last newline.
fn visible_prompt(prompt: &str) -> &str {
    match prompt.rfind('\n') {
        Some(i) => &prompt[i + 1..],
        None => prompt,
    }
}

/// Cells a string occupies on screen, skipping ANSI escape sequences.
///
/// A styled prompt such as "\x1b[36m>\x1b[0m " paints two cells, but its bytes include
/// the escape sequences. Measuring those as printable is what made every repositioning
/// land seven columns too far right, which showed up as an unerasable gap after a
/// backspace: the fast path for plain typing never repositions, so only a redraw exposed it.
fn visible_width(s: &str) -> usize {
    let mut cols = 0usize;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            cols += c.width().unwrap_or(0);
            continue;
        }
        // CSI: ESC '[' parameters, terminated by a byte in @ to ~.
        if it.peek() == Some(&'[') {
            it.next();
            for t in it.by_ref() {
                if ('\x40'..='\x7e').contains(&t) {
                    break;
                }
            }
        } else {
            // A non CSI escape: any intermediate bytes in 0x20 to 0x2F, then one final
            // byte. "ESC ( B" is three characters, so dropping a fixed two would leave
            // the 'B' behind and count it as text.
            while it.peek().is_some_and(|c| ('\x20'..='\x2f').contains(c)) {
                it.next();
            }
            it.next();
        }
    }
    cols
}

/// Display width of the buffer up to a content cursor.
fn width_upto(buf: &[char], cursor: usize) -> usize {
    buf[..cursor.min(buf.len())].iter().map(|c| c.width().unwrap_or(0)).sum()
}

/// The screen row a content cursor sits on, counted from the first row of the input area.
fn row_of(prompt_cols: usize, buf: &[char], cursor: usize) -> usize {
    row_col(prompt_cols, buf, cursor).0
}

/// The row and column a content cursor sits at, given the prompt width.
fn row_col(prompt_cols: usize, buf: &[char], cursor: usize) -> (usize, usize) {
    let off = prompt_cols + width_upto(buf, cursor);
    let w = term_cols();
    (off / w, off % w)
}

/// Park the physical cursor at the end of the content, ready for a newline.
fn move_to_end(prompt_cols: usize, buf: &[char], cursor: usize) {
    let (cur_row, _) = row_col(prompt_cols, buf, cursor);
    let (end_row, end_col) = row_col(prompt_cols, buf, buf.len());
    let mut out = std::io::stdout();
    if end_row > cur_row {
        let _ = write!(out, "\x1b[{}B", end_row - cur_row);
    }
    let _ = write!(out, "\x1b[{}G", end_col + 1);
    let _ = out.flush();
}

/// Erase the whole input area and repaint it, leaving the cursor at `to`.
///
/// `from_row` is where the physical cursor is RIGHT NOW, measured by the caller against the
/// buffer as it was BEFORE the edit. Deriving it here from the new buffer would be wrong:
/// the widths have already shifted while the terminal cursor has not moved.
///
/// Content is written row by row with explicit `\r\n` at every wrap boundary. Relying on
/// the terminal's soft-wrap is unsound: after a soft-wrap the cursor is on a new physical
/// line, but `\x1b[nG` (column absolute) moves within the *current* row only. Writing the
/// breaks ourselves means the cursor position after the repaint is always precisely known.
fn redraw(prompt: &str, prompt_cols: usize, buf: &[char], from_row: usize, to: usize) {
    let mut out = std::io::stdout();
    let w = term_cols();

    // Back to the first row of the input area, then clear everything below.
    if from_row > 0 {
        let _ = write!(out, "\x1b[{from_row}A");
    }
    let _ = write!(out, "\r\x1b[0J");

    // Write the visible prompt (the last line of the prompt string).
    let _ = write!(out, "{}", visible_prompt(prompt));

    // Write the buffer character by character, emitting an explicit \r\n whenever a
    // character would overflow the current row. This keeps the terminal cursor position
    // in sync with what row_col() computes: both use the same integer division model.
    let mut col = prompt_cols;
    for &c in buf {
        let cw = c.width().unwrap_or(0);
        if col + cw > w {
            let _ = write!(out, "\r\n");
            col = 0;
        }
        let _ = write!(out, "{c}");
        col += cw;
    }

    // Reposition: after the loop the cursor sits at the end of the content.
    // Use the same row_col() model to find the target position, then move there.
    let (end_row, _) = row_col(prompt_cols, buf, buf.len());
    let (target_row, target_col) = row_col(prompt_cols, buf, to);
    if end_row > target_row {
        let _ = write!(out, "\x1b[{}A", end_row - target_row);
    }
    let _ = write!(out, "\x1b[{}G", target_col + 1);
    let _ = out.flush();
}

/// Move the cursor without touching the content.
fn move_cursor(prompt_cols: usize, buf: &[char], from: usize, to: usize) {
    let (from_row, _) = row_col(prompt_cols, buf, from);
    let (to_row, to_col) = row_col(prompt_cols, buf, to);
    let mut out = std::io::stdout();
    if to_row < from_row {
        let _ = write!(out, "\x1b[{}A", from_row - to_row);
    } else if to_row > from_row {
        let _ = write!(out, "\x1b[{}B", to_row - from_row);
    }
    let _ = write!(out, "\x1b[{}G", to_col + 1);
    let _ = out.flush();
}

/// Swap in a history entry, leaving the cursor at the end of the new text.
///
/// `from_row` comes from the CURRENT cursor, not the old end. The reference implementation
/// used the old end, which over scrolled whenever the cursor was not already there, for
/// instance Home followed by Up on a wrapped line.
fn replace_buffer(
    prompt: &str,
    prompt_cols: usize,
    buf: &mut Vec<char>,
    cursor: &mut usize,
    text: &str,
    from_row: usize,
) {
    buf.clear();
    buf.extend(text.chars());
    *cursor = buf.len();
    // The cursor ends at the end of the content, which is exactly where the repaint leaves
    // it, so this is the same operation as a redraw targeting the end.
    redraw(prompt, prompt_cols, buf, from_row, *cursor);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_width_counts_wide_characters_as_two() {
        assert_eq!(visible_width("abc"), 3);
        assert_eq!(visible_width("❯ "), 2, "the prompt glyph is one cell");
        assert_eq!(visible_width("漢字"), 4, "each CJK char takes two cells");
    }

    /// The bug behind the unerasable gap after a backspace: the colour escapes in the
    /// prompt were measured as printable cells, so every repositioning landed seven
    /// columns too far right. Plain typing never repositions, which is why only a redraw
    /// showed it.
    #[test]
    fn visible_width_ignores_ansi_escapes_in_the_prompt() {
        let styled = "\x1b[36m❯\x1b[0m ";
        assert_eq!(visible_width(styled), 2, "two painted cells, not nine");
        // The naive measure is what used to be used, and it is off by exactly seven.
        let naive: usize = styled.chars().map(|c| c.width().unwrap_or(0)).sum();
        assert_eq!(naive, 9);
        assert_eq!(naive - visible_width(styled), 7, "the size of the gap that appeared");
    }

    #[test]
    fn visible_width_handles_the_escape_forms_that_appear_in_practice() {
        assert_eq!(visible_width("\x1b[1;4;97mtitle\x1b[0m"), 5, "multi parameter CSI");
        assert_eq!(visible_width("\x1b[0Kx"), 1, "a non colour CSI still occupies nothing");
        assert_eq!(visible_width("\x1b(Bx"), 1, "a two character escape");
        assert_eq!(visible_width("\x1b[36m"), 0, "a bare escape paints nothing");
        assert_eq!(visible_width("a\x1b[38;5;208mb"), 2, "text either side is counted");
    }

    #[test]
    fn visible_prompt_keeps_only_the_last_visual_line() {
        assert_eq!(visible_prompt("\n\x1b[36m❯\x1b[0m "), "\x1b[36m❯\x1b[0m ");
        assert_eq!(visible_prompt("  "), "  ", "a continuation prompt has no newline");
        assert_eq!(visible_prompt("a\nb\nc"), "c");
    }

    /// The whole point of the fix: with a styled prompt the geometry must match the two
    /// cells actually painted, so a redraw puts the cursor back on the text.
    #[test]
    fn the_geometry_of_a_styled_prompt_matches_what_is_painted() {
        let prompt = "\n\x1b[36m❯\x1b[0m ";
        let cols = visible_width(visible_prompt(prompt));
        assert_eq!(cols, 2);

        let buf: Vec<char> = "hello".chars().collect();
        // Five characters after a two cell prompt end at column 7, on row 0.
        assert_eq!(row_col(cols, &buf, buf.len()), (0, 7));
        // And after deleting one, at column 6, not 13.
        let shorter: Vec<char> = "hell".chars().collect();
        assert_eq!(row_col(cols, &shorter, shorter.len()), (0, 6));
    }

    #[test]
    fn row_of_agrees_with_row_col() {
        let buf: Vec<char> = "x".repeat(200).chars().collect();
        for cursor in [0, 10, 79, 80, 199, 200] {
            assert_eq!(row_of(2, &buf, cursor), row_col(2, &buf, cursor).0, "at {cursor}");
        }
    }

    #[test]
    fn row_col_wraps_on_the_terminal_width() {
        let buf: Vec<char> = "x".repeat(100).chars().collect();
        let w = term_cols();
        // Position 0 is on row 0 right after the prompt.
        assert_eq!(row_col(2, &buf, 0), (0, 2));
        // A cursor past the width lands on the next row.
        let (row, col) = row_col(2, &buf, w);
        assert_eq!((row, col), (1, 2), "prompt of 2 pushes the wrap by two cells");
    }

    #[test]
    fn row_col_accounts_for_wide_characters() {
        let buf: Vec<char> = "漢漢漢".chars().collect();
        // Three CJK chars are six cells, so a cursor after all three sits at column 6.
        assert_eq!(row_col(0, &buf, 3).1, 6);
        assert_eq!(width_upto(&buf, 2), 4);
    }

    #[test]
    fn width_upto_is_clamped_to_the_buffer() {
        let buf: Vec<char> = "abc".chars().collect();
        assert_eq!(width_upto(&buf, 99), 3, "an out of range cursor does not panic");
    }

    #[test]
    fn history_skips_blanks_and_immediate_repeats() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("readline.txt");
        let mut r = InputReader::new(&f);
        r.add_history("first");
        r.add_history("first");
        r.add_history("   ");
        r.add_history("second");
        assert_eq!(r.history, ["first", "second"]);

        // And it is persisted, so a new reader sees it.
        let again = InputReader::new(&f);
        assert_eq!(again.history, ["first", "second"]);
    }

    #[test]
    fn history_loading_ignores_blank_lines_and_applies_the_cap() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("h.txt");
        let mut body = String::new();
        for i in 0..HISTORY_CAP + 50 {
            body.push_str(&format!("line{i}\n\n"));
        }
        std::fs::write(&f, body).unwrap();
        let r = InputReader::new(&f);
        assert_eq!(r.history.len(), HISTORY_CAP, "only the tail is kept");
        assert_eq!(r.history.last().unwrap(), &format!("line{}", HISTORY_CAP + 49));
    }

    #[test]
    fn a_missing_history_file_starts_empty() {
        let d = tempfile::tempdir().unwrap();
        let r = InputReader::new(&d.path().join("nope.txt"));
        assert!(r.history.is_empty());
    }

    #[test]
    fn save_trims_an_oversized_history_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("h.txt");
        let mut r = InputReader::new(&f);
        for i in 0..HISTORY_CAP + 10 {
            r.history.push(format!("l{i}"));
        }
        r.save();
        let kept = std::fs::read_to_string(&f).unwrap().lines().count();
        assert_eq!(kept, HISTORY_CAP);
    }

    #[test]
    fn the_prompt_width_only_counts_the_last_visual_line() {
        // "\n❯ " puts the glyph on a fresh row, so only two cells share the cursor row.
        assert_eq!(visible_width(visible_prompt("\n❯ ")), 2);
    }

    #[test]
    fn term_cols_never_returns_a_degenerate_width() {
        assert!(term_cols() >= 20, "a tiny or unknown terminal still yields usable maths");
    }

    /// Verify that the explicit-newline repaint model keeps geometry consistent when the
    /// buffer overflows the terminal width. `row_col` must agree with a manual calculation
    /// on a known-width buffer regardless of soft-wrap.
    #[test]
    fn redraw_geometry_is_consistent_across_wrap_boundary() {
        let w = term_cols(); // at least 20 per the invariant above
        let prompt_cols = 2usize;

        // A buffer long enough to span two full rows.
        let buf: Vec<char> = "x".repeat(w * 2 + 5).chars().collect();

        // Manual calculation matching the explicit-newline model:
        //   offset = prompt_cols + width_upto(buf, cursor)
        //   row    = offset / w
        //   col    = offset % w
        for &cursor in &[0, w - prompt_cols, w - prompt_cols + 1, w * 2, buf.len()] {
            let cursor = cursor.min(buf.len());
            let offset = prompt_cols + width_upto(&buf, cursor);
            let expected_row = offset / w;
            let expected_col = offset % w;
            let (got_row, got_col) = row_col(prompt_cols, &buf, cursor);
            assert_eq!(
                (got_row, got_col),
                (expected_row, expected_col),
                "geometry mismatch at cursor={cursor}"
            );
        }

        // The geometry functions must not overflow or panic for a wrapped buffer.
        let (end_row, _end_col) = row_col(prompt_cols, &buf, buf.len());
        let (target_row, target_col) = row_col(prompt_cols, &buf, w - prompt_cols + 1);
        assert!(end_row >= target_row, "end must be at or after target");
        assert!(target_col < w, "target column must be within terminal width");
    }
}
