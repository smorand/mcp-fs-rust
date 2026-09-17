//! Fixed-window text chunker with sentence-boundary tolerance.
//!
//! Splits text into overlapping windows of at most `size` characters. When the
//! natural cut point falls inside a word, the chunker looks back up to 50 chars
//! for a sentence boundary (`.`, `\n`, or space) to produce a cleaner split.
//! Overlap carries the last `overlap` characters of the previous chunk into the
//! next one, so the embedding captures cross-boundary context.

/// Split `text` into chunks of at most `size` chars with `overlap` char overlap.
/// Returns an empty vec when the text is empty.
pub fn chunk(text: &str, size: usize, overlap: usize) -> Vec<String> {
    if text.is_empty() || size == 0 {
        return Vec::new();
    }
    // Clamp overlap so it cannot exceed size, which would cause an infinite loop.
    let overlap = overlap.min(size.saturating_sub(1));

    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();

    if total <= size {
        return vec![chars.iter().collect()];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;

    while start < total {
        let end_raw = (start + size).min(total);

        // Try to find a sentence boundary in the last 50 chars before end_raw so
        // we do not cut mid-word. Only look back when there is room.
        let end = if end_raw < total {
            let look_back = end_raw.saturating_sub(50);
            let boundary = chars[look_back..end_raw]
                .iter()
                .enumerate()
                .rev()
                .find(|(_, c)| matches!(**c, '.' | '\n' | ' '))
                .map(|(i, _)| look_back + i + 1);
            boundary.unwrap_or(end_raw)
        } else {
            end_raw
        };

        // Guard: never emit an empty chunk or regress the cursor.
        let end = end.max(start + 1).min(total);
        chunks.push(chars[start..end].iter().collect());

        if end >= total {
            break;
        }

        // Advance by (end - start - overlap), but at least 1 to avoid stalling.
        let step = (end - start).saturating_sub(overlap).max(1);
        start += step;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_respects_size() {
        // A 10 char string chunked at size 4, no overlap gives 3 chunks.
        let text = "abcdefghij";
        let chunks = chunk(text, 4, 0);
        // Every chunk must be at most 4 chars.
        for c in &chunks {
            assert!(c.len() <= 4, "chunk too long: {c:?}");
        }
        // The concatenation must cover the whole input (with no overlap, no repetition).
        let joined: String = chunks.join("");
        assert_eq!(joined, text);
    }

    #[test]
    fn chunk_overlap_carries_context() {
        // A 20 char string chunked at size 8 with overlap 3: each chunk after the
        // first must share the last 3 chars of the previous.
        let text = "abcdefghijklmnopqrst";
        let chunks = chunk(text, 8, 3);
        assert!(chunks.len() >= 2, "need at least two chunks to check overlap");
        for pair in chunks.windows(2) {
            let prev_tail: String =
                pair[0].chars().rev().take(3).collect::<String>().chars().rev().collect();
            let next_head: String = pair[1].chars().take(3).collect();
            assert_eq!(
                prev_tail, next_head,
                "overlap not carried: prev tail={prev_tail:?}, next head={next_head:?}"
            );
        }
    }

    #[test]
    fn empty_text_returns_no_chunks() {
        assert!(chunk("", 100, 0).is_empty());
    }

    #[test]
    fn short_text_returns_one_chunk() {
        assert_eq!(chunk("hello", 100, 0), vec!["hello".to_string()]);
    }

    #[test]
    fn size_zero_returns_no_chunks() {
        assert!(chunk("hello", 0, 0).is_empty());
    }
}
