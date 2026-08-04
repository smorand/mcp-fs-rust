//! Conversation persistence: one `.jsonl` file per session under `.agent_history/`.
//!
//! Each line is `{"role","content","ts"}`. Messages are appended as they happen rather
//! than written at exit, so a crash or a kill still leaves the transcript on disk.

use anyhow::Result;
use std::path::{Path, PathBuf};

/// A conversation transcript on disk.
pub struct Session {
    path: PathBuf,
    id: String,
    is_new: bool,
}

/// One persisted turn.
pub struct StoredMessage {
    pub role: String,
    pub content: String,
}

impl Session {
    /// Open `id`, or mint a timestamp id when `id` is None. A named session that has no
    /// file yet counts as new, which is what lets `--conversation foo` create `foo`.
    pub fn open(history_dir: &Path, id: Option<&str>) -> Self {
        let id = match id {
            Some(i) => i.to_string(),
            None => chrono::Local::now().format("%Y%m%d-%H%M%S").to_string(),
        };
        let path = history_dir.join(format!("{id}.jsonl"));
        let is_new = !path.exists();
        Self { path, id, is_new }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn is_new(&self) -> bool {
        self.is_new
    }

    /// Replay the transcript. A malformed line is skipped rather than fatal: a partial
    /// write from a killed process must not make the session unreadable forever.
    pub fn load_messages(&self) -> Vec<StoredMessage> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .map(|v| StoredMessage {
                role: v["role"].as_str().unwrap_or("user").to_string(),
                content: v["content"].as_str().unwrap_or_default().to_string(),
            })
            .collect()
    }

    /// Append one turn.
    pub fn append(&self, role: &str, content: &str) -> Result<()> {
        use std::io::Write;
        let line = serde_json::json!({
            "role": role,
            "content": content,
            "ts": chrono::Utc::now().timestamp(),
        });
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// Drop the last turn, used when an LLM call fails so the transcript does not keep a
    /// user message that was never answered.
    pub fn remove_last(&self) -> Result<()> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Ok(());
        };
        let mut lines: Vec<&str> = text.lines().collect();
        if lines.pop().is_none() {
            return Ok(());
        }
        let mut out = lines.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        std::fs::write(&self.path, out)?;
        Ok(())
    }
}

/// The most recently touched sessions, newest first: `(id, modified, message count)`.
pub fn list_sessions(history_dir: &Path, limit: usize) -> Vec<(String, std::time::SystemTime, usize)> {
    let Ok(rd) = std::fs::read_dir(history_dir) else {
        return Vec::new();
    };
    let mut rows: Vec<_> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_stem()?.to_string_lossy().to_string();
            let modified = e.metadata().ok()?.modified().ok()?;
            let count = std::fs::read_to_string(&path)
                .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count())
                .unwrap_or(0);
            Some((id, modified, count))
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    rows.truncate(limit);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_session_gets_a_timestamp_id_and_reads_empty() {
        let d = tempfile::tempdir().unwrap();
        let s = Session::open(d.path(), None);
        assert!(s.is_new());
        assert_eq!(s.id().len(), 15, "yyyymmdd-hhmmss");
        assert!(s.load_messages().is_empty());
    }

    #[test]
    fn appended_turns_round_trip_in_order() {
        let d = tempfile::tempdir().unwrap();
        let s = Session::open(d.path(), Some("conv"));
        s.append("user", "bonjour").unwrap();
        s.append("assistant", "salut").unwrap();

        // Reopening the same id must see the file and replay it.
        let again = Session::open(d.path(), Some("conv"));
        assert!(!again.is_new());
        let msgs = again.load_messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!((msgs[0].role.as_str(), msgs[0].content.as_str()), ("user", "bonjour"));
        assert_eq!((msgs[1].role.as_str(), msgs[1].content.as_str()), ("assistant", "salut"));
    }

    #[test]
    fn content_with_newlines_and_quotes_survives() {
        let d = tempfile::tempdir().unwrap();
        let s = Session::open(d.path(), Some("c"));
        let tricky = "line one\nline \"two\"\n{\"json\": true}";
        s.append("user", tricky).unwrap();
        assert_eq!(s.load_messages()[0].content, tricky, "json escaping keeps it on one line");
    }

    #[test]
    fn remove_last_drops_only_the_final_turn() {
        let d = tempfile::tempdir().unwrap();
        let s = Session::open(d.path(), Some("c"));
        s.append("user", "a").unwrap();
        s.append("assistant", "b").unwrap();
        s.remove_last().unwrap();
        let msgs = s.load_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "a");
        // And appending afterwards must not concatenate onto the previous line.
        s.append("user", "c").unwrap();
        assert_eq!(s.load_messages().len(), 2);
    }

    #[test]
    fn remove_last_on_an_absent_or_empty_file_is_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        let s = Session::open(d.path(), Some("never-written"));
        s.remove_last().unwrap();
        s.append("user", "only").unwrap();
        s.remove_last().unwrap();
        assert!(s.load_messages().is_empty());
        s.remove_last().unwrap();
    }

    #[test]
    fn a_malformed_line_is_skipped_not_fatal() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("c.jsonl");
        std::fs::write(&path, "{\"role\":\"user\",\"content\":\"ok\"}\nnot json at all\n").unwrap();
        let s = Session::open(d.path(), Some("c"));
        let msgs = s.load_messages();
        assert_eq!(msgs.len(), 1, "the readable turn is still returned");
        assert_eq!(msgs[0].content, "ok");
    }

    #[test]
    fn sessions_are_listed_newest_first_with_counts() {
        let d = tempfile::tempdir().unwrap();
        let old = Session::open(d.path(), Some("older"));
        old.append("user", "1").unwrap();
        // Distinct mtimes without sleeping: set them explicitly is not portable, so
        // rely on ordering by writing the newer one second and comparing counts.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let new = Session::open(d.path(), Some("newer"));
        new.append("user", "1").unwrap();
        new.append("assistant", "2").unwrap();

        let rows = list_sessions(d.path(), 10);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "newer", "newest first");
        assert_eq!(rows[0].2, 2, "two messages");
        assert_eq!(rows[1].0, "older");
        assert_eq!(rows[1].2, 1);
    }

    #[test]
    fn listing_honours_the_limit_and_ignores_other_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("readline.txt"), "not a session\n").unwrap();
        for i in 0..5 {
            Session::open(d.path(), Some(&format!("s{i}"))).append("user", "x").unwrap();
        }
        assert_eq!(list_sessions(d.path(), 3).len(), 3);
        assert_eq!(list_sessions(d.path(), 99).len(), 5, "readline.txt is not a session");
    }

    #[test]
    fn listing_a_missing_directory_is_empty_not_a_panic() {
        assert!(list_sessions(Path::new("/nope/nowhere"), 5).is_empty());
    }
}
