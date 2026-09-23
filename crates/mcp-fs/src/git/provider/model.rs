//! The one normalized pull request object every `git.pr_*` tool returns
//! (FR-NEW-303, FR-NEW-317, DEC-906).
//!
//! Defined ONCE, as a type, because the audit of this surface caught wire shape
//! drift four times when each call site hand built its own JSON: a folded
//! `draft`, a missing `provider`, a reordered key. [`PullRequest`] is a struct
//! whose field order IS the key order (`serde_json` is built with
//! `preserve_order`, so a struct serializes in declaration order), and both
//! providers map into it through [`PullRequest::from_github`] and
//! [`PullRequest::from_gitlab`]. A tool never assembles these keys itself.
//!
//! `raw` carries the provider's payload untouched, so normalization loses
//! nothing. It is the response the transport already scrubbed of the
//! credential, so no request authorization data can ride along in it.

use chrono::{DateTime, FixedOffset};
use serde::Serialize;
use serde_json::Value;

use super::PrProvider;

/// A GitLab merge request has no draft flag: the marker is a title prefix, so
/// the prefix is stripped on the way in and `draft` carries the fact instead.
/// That is what makes the two providers agree on `title` (E2E-NEW-721,
/// E2E-NEW-737).
const GITLAB_DRAFT_PREFIXES: [&str; 2] = ["Draft: ", "WIP: "];

/// What is known about reviews and checks for one pull request.
///
/// Separate from the payload because both providers report them on OTHER
/// endpoints (`/reviews` and `/check-runs`, `/approvals`): a tool that did not
/// ask for them passes [`PrSignals::unknown`] and the normalized object says so
/// rather than inventing an answer.
#[derive(Debug, Clone, Default)]
pub struct PrSignals {
    pub review_state: Option<String>,
    pub checks_state: Option<String>,
}

impl PrSignals {
    /// Neither endpoint was consulted: `review_state` is `none` (no review is
    /// recorded on a pull request that was just created) and `checks_state` is
    /// `unknown` (a check may well exist; this call did not look).
    pub fn unknown() -> Self {
        Self::default()
    }

    fn review(&self) -> String {
        self.review_state.clone().unwrap_or_else(|| "none".to_string())
    }

    fn checks(&self) -> String {
        self.checks_state.clone().unwrap_or_else(|| "unknown".to_string())
    }
}

/// The normalized pull request. Field order is the frozen key order of
/// FR-NEW-317; do not reorder, a caller asserts on it.
#[derive(Debug, Clone, Serialize)]
pub struct PullRequest {
    pub provider: String,
    pub host: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    /// `open`, `closed` or `merged`. Never carries draftness: that is `draft`.
    pub state: String,
    pub draft: bool,
    pub base: String,
    pub head: String,
    pub author: String,
    pub url: String,
    pub created_at: String,
    pub updated_at: String,
    pub commits: u64,
    pub changed_files: u64,
    pub additions: u64,
    pub deletions: u64,
    pub review_state: String,
    pub checks_state: String,
    /// Tri state: `None` serializes to JSON `null`, meaning the provider has
    /// not computed mergeability yet. Never collapsed to `false`.
    pub mergeable: Option<bool>,
    pub raw: Value,
}

impl PullRequest {
    /// Map a GitHub pull request payload. `raw` is moved in, not cloned: every
    /// field is read through a borrow first, then the same `Value` becomes
    /// `raw`.
    pub fn from_github(host: &str, raw: Value, signals: &PrSignals) -> Self {
        let merged = raw.get("merged_at").is_some_and(|v| !v.is_null())
            || raw.get("merged").and_then(Value::as_bool).unwrap_or(false);
        let state = if merged {
            "merged"
        } else if text(&raw, "state") == "closed" {
            "closed"
        } else {
            "open"
        };
        Self {
            provider: PrProvider::Github.label().to_string(),
            host: host.to_string(),
            number: count(raw.get("number")),
            title: text(&raw, "title"),
            body: text(&raw, "body"),
            state: state.to_string(),
            draft: raw.get("draft").and_then(Value::as_bool).unwrap_or(false),
            base: nested(&raw, "base", "ref"),
            head: nested(&raw, "head", "ref"),
            author: nested(&raw, "user", "login"),
            url: text(&raw, "html_url"),
            created_at: timestamp(&raw, "created_at"),
            updated_at: timestamp(&raw, "updated_at"),
            commits: count(raw.get("commits")),
            changed_files: count(raw.get("changed_files")),
            additions: count(raw.get("additions")),
            deletions: count(raw.get("deletions")),
            review_state: signals.review(),
            checks_state: signals.checks(),
            // Present and null while GitHub computes it: that is the tri state,
            // and it must not become `false`.
            mergeable: raw.get("mergeable").and_then(Value::as_bool),
            raw,
        }
    }

    /// Map a GitLab merge request payload into the identical shape.
    pub fn from_gitlab(host: &str, raw: Value, signals: &PrSignals) -> Self {
        let state = match text(&raw, "state").as_str() {
            "merged" => "merged",
            "closed" | "locked" => "closed",
            _ => "open",
        };
        let raw_title = text(&raw, "title");
        // One pass: the stripped title AND the fact there was a prefix, without
        // cloning the title a second time.
        let (title, prefixed) =
            match GITLAB_DRAFT_PREFIXES.iter().find_map(|p| raw_title.strip_prefix(p)) {
                Some(stripped) => (stripped.to_string(), true),
                None => (raw_title, false),
            };
        Self {
            provider: PrProvider::Gitlab.label().to_string(),
            host: host.to_string(),
            number: count(raw.get("iid")),
            title,
            body: text(&raw, "description"),
            state: state.to_string(),
            // The title prefix is a draft marker in its own right: a GitLab
            // deployment old enough to have no `draft` field still reports it.
            draft: raw.get("draft").and_then(Value::as_bool).unwrap_or(prefixed),
            base: text(&raw, "target_branch"),
            head: text(&raw, "source_branch"),
            author: nested(&raw, "author", "username"),
            url: text(&raw, "web_url"),
            created_at: timestamp(&raw, "created_at"),
            updated_at: timestamp(&raw, "updated_at"),
            commits: count(raw.get("commits_count")),
            // `changes_count` is a STRING on GitLab ("4", and "1000+" past the
            // cap), so it goes through the same tolerant reader as every count.
            changed_files: count(raw.get("changes_count")),
            additions: count(diff_stat(&raw, "additions")),
            deletions: count(diff_stat(&raw, "deletions")),
            review_state: signals.review(),
            checks_state: signals.checks(),
            mergeable: match text(&raw, "merge_status").as_str() {
                "can_be_merged" => Some(true),
                "cannot_be_merged" => Some(false),
                // `checking`, `unchecked` and an absent field all mean the
                // provider has not answered yet.
                _ => None,
            },
            raw,
        }
    }

    /// The tool response. One place, so no call site can drop or add a key.
    ///
    /// Serialization cannot fail: every field is a string, a number, a bool or
    /// an already valid [`Value`], and there is no map with non string keys.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// A string field, with JSON `null` and an absent key both meaning empty: a
/// pull request with no body must not turn into the literal text "null".
fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn nested(v: &Value, outer: &str, inner: &str) -> String {
    v.get(outer).map(|o| text(o, inner)).unwrap_or_default()
}

/// GitLab reports line counts under `diff_stats_summary`, falling back to the
/// flat key a merge request from an older deployment carries.
fn diff_stat<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get("diff_stats_summary").and_then(|s| s.get(key)).or_else(|| v.get(key))
}

/// A count, from a JSON number or from a numeric string. Anything else is 0:
/// a count is a count, and a payload that omits it (a list entry, say) reports
/// zero rather than failing the whole call.
fn count(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.trim_end_matches('+').parse().unwrap_or(0),
        _ => 0,
    }
}

/// One timestamp spelling for both providers: GitHub sends `...Z`, GitLab sends
/// `....000Z`, and a caller comparing the two must see the same string. An
/// unparseable value is passed through rather than dropped.
fn timestamp(v: &Value, key: &str) -> String {
    let raw = text(v, key);
    match DateTime::<FixedOffset>::parse_from_rfc3339(&raw) {
        Ok(t) => t.to_utc().to_rfc3339(),
        Err(_) => raw,
    }
}
