//! Web search tools: DuckDuckGo search, news, suggestions, page fetch, and binary download.
//!
//! These tools require no API key. They are registered only when `web.enabled`
//! is true.
//!
//! Most tools are stateless: they return content to the LLM, which then decides
//! what to do with it. Two tools can also write directly into a volume:
//! - `web.fetch` accepts optional `mount_id`+`save_path` to bypass the context
//!   window entirely for large pages or when the raw HTML is needed.
//! - `web.download` always writes binary content (images, PDFs, ZIPs...) to a
//!   volume; binary data cannot be passed through the LLM context.
//!
//! When writing to a volume the caller is subject to the same quota and ACL
//! rules as `fs.write`: the bearer token must be a member of the project and
//! the session write quota applies.

use crate::core::fs_ops;
use crate::errors::ToolError;
use crate::tools::registry_support::handler;
use crate::tools::registry_support::{ToolRegistry, ToolSchema};
use crate::tools::{norm, volume};
use serde_json::{Value, json};

/// Register the four `web.*` tools.
pub fn register(reg: &mut ToolRegistry, config: &crate::config::WebConfig) {
    reg.add(
        ToolSchema::new(
            "web.search",
            "Search the web using DuckDuckGo. Returns a list of results with title, URL, and snippet.",
        )
        .req_str("query", "Search query.")
        .opt_int("max_results", 10, "Maximum number of results to return (capped at 50).")
        .opt_str("safe_search", "moderate", "Safe search level: off, moderate, or strict.")
        .read_only(true)
        .idempotent(true)
        .open_world(true),
        {
            let config = config.clone();
            handler(move |_ctx, a| {
                let config = config.clone();
                async move {
                    let query = a.str("query")?;
                    let max = cap_results(a.int_or("max_results", 0), 10);
                    let safe = a.str_or("safe_search", "moderate");
                    web_search(&endpoints(&config), &query, max, &safe).await
                }
            })
        },
    );

    reg.add(
        ToolSchema::new("web.news", "Search recent news using DuckDuckGo.")
            .req_str("query", "News search query.")
            .opt_int("max_results", 10, "Maximum number of results to return (capped at 50).")
            .opt_str_null(
                "time_range",
                "Time range filter: d (day), w (week), m (month), y (year).",
            )
            .read_only(true)
            .idempotent(true)
            .open_world(true),
        {
            let config = config.clone();
            handler(move |_ctx, a| {
                let config = config.clone();
                async move {
                    let query = a.str("query")?;
                    let max = cap_results(a.int_or("max_results", 0), 10);
                    let time_range = a.opt_str("time_range");
                    web_news(&endpoints(&config), &query, max, time_range.as_deref()).await
                }
            })
        },
    );

    reg.add(
        ToolSchema::new(
            "web.fetch",
            "Fetch the content of a web page. \
             By default strips HTML tags, caps at 50 000 chars, and returns the text to the \
             LLM (good for summarising or extracting information). \
             Provide mount_id + save_path to write the raw content directly into a volume \
             instead: use this when the page is large, when you need the original HTML, or \
             when the content should be persisted without consuming context window tokens.",
        )
        .req_str("url", "URL to fetch (must start with http:// or https://).")
        .opt_int("timeout_secs", 10, "Request timeout in seconds (capped at 30).")
        .opt_str_null(
            "mount_id",
            "If set together with save_path, write the fetched content into this volume \
             instead of returning it to the LLM.",
        )
        .opt_str_null(
            "save_path",
            "Absolute POSIX destination path within the volume (requires mount_id).",
        )
        .read_only(true)
        .idempotent(true)
        .open_world(true),
        handler(|ctx, a| async move {
            let url = a.str("url")?;
            let timeout = a.int_or("timeout_secs", 10).clamp(1, 30) as u64;
            let mount_id = a.opt_str("mount_id");
            let save_path = a.opt_str("save_path");
            match (mount_id, save_path) {
                (Some(_), None) | (None, Some(_)) => Err(ToolError::invalid_argument(
                    "mount_id and save_path must both be provided, or neither",
                )),
                (Some(mid), Some(sp)) => {
                    // Write the raw bytes directly into the volume; the content never
                    // passes through the LLM context window.
                    let fake_a = crate::tools::registry_support::Args::new(
                        json!({"mount_id": mid, "path": sp}),
                    );
                    let (mount, client) = volume(&ctx, &fake_a).await?;
                    let path = norm(&ctx, &fake_a, "path")?;
                    let (bytes, _content_type) =
                        fetch_raw(&url, timeout, u64::MAX, unreachable_over).await?;
                    fs_ops::write_bytes(
                        &client,
                        &ctx.state.safety,
                        &ctx.person,
                        &mount,
                        &path,
                        &bytes,
                        true,
                        true,
                    )
                    .await
                }
                (None, None) => web_fetch(&url, timeout).await,
            }
        }),
    );

    reg.add(
        ToolSchema::new(
            "web.download",
            "Download a URL and write the raw bytes directly into a volume file. \
             Unlike web.fetch this tool does not pass content through the LLM context window, \
             making it the right choice for images, PDFs, ZIPs, and other binary files. \
             The session write quota and project ACL apply exactly as for fs.write.",
        )
        .req_str("url", "URL to download (must start with http:// or https://).")
        .req_str("mount_id", "Project/volume id to write into.")
        .req_str("path", "Absolute POSIX destination path within the volume.")
        .opt_int("timeout_secs", 30, "Request timeout in seconds (capped at 120).")
        .destructive(true)
        .read_only(false)
        .idempotent(true)
        .open_world(true),
        handler(|ctx, a| async move {
            let url = a.str("url")?;
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(ToolError::invalid_argument("url must start with http:// or https://"));
            }
            let timeout = a.int_or("timeout_secs", 30).clamp(1, 120) as u64;
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            let (bytes, content_type) =
                fetch_raw(&url, timeout, u64::MAX, unreachable_over).await?;
            let mut result = fs_ops::write_bytes(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &bytes,
                true,
                true,
            )
            .await?;
            if let Some(obj) = result.as_object_mut() {
                obj.insert("content_type".to_string(), Value::String(content_type));
            }
            Ok(result)
        }),
    );

    reg.add(
        ToolSchema::new(
            "web.suggestions",
            "Get search query autocomplete suggestions from DuckDuckGo.",
        )
        .req_str("query", "Partial query to complete.")
        .read_only(true)
        .idempotent(true)
        .open_world(true),
        {
            let config = config.clone();
            handler(move |_ctx, a| {
                let config = config.clone();
                async move {
                    let query = a.str("query")?;
                    web_suggestions(&endpoints(&config), &query).await
                }
            })
        },
    );
}

/// Catalog-only, `ToolRegistry`-free view of the `web.*` family, for consumers
/// that only need name/description/schema/annotations (e.g. `/api/swagger.json`).
pub fn catalog(config: &crate::config::WebConfig) -> Vec<crate::tools::catalog::ToolCatalogEntry> {
    let mut reg = ToolRegistry::new();
    register(&mut reg, config);
    reg.names()
        .iter()
        .map(|n| crate::tools::catalog::ToolCatalogEntry::from(&reg.resolve(n).unwrap().schema))
        .collect()
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Map safe_search string to the DuckDuckGo `kp` parameter value.
fn safe_param(safe: &str) -> &'static str {
    match safe {
        "strict" => "1",
        "off" => "-1",
        _ => "0",
    }
}

/// Cap the requested result count. `0` (or negative) means use the default.
fn cap_results(requested: i64, default: usize) -> usize {
    if requested <= 0 { default } else { (requested as usize).min(50) }
}

/// Strip HTML tags and decode common HTML entities from a string.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => {
                in_tag = true;
            }
            '>' => {
                in_tag = false;
            }
            _ if !in_tag => {
                out.push(ch);
            }
            _ => {}
        }
    }
    // Decode entities after stripping tags.
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn build_client(timeout_secs: u64) -> Result<reqwest::Client, ToolError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .user_agent("mcp-fs-web/1.0")
        .build()
        .map_err(|e| ToolError::internal(format!("http client error: {e}")))
}

/// Extract text between a delimiter and the next `<` (or end of string), stripping tags.
fn extract_between(haystack: &str, after: &str) -> Option<String> {
    let start = haystack.find(after)? + after.len();
    let slice = &haystack[start..];
    // Find end: the enclosing `</a>` or `</span>` — just go to the first `</`.
    let end = slice.find("</").unwrap_or(slice.len());
    let raw = &slice[..end];
    let text = strip_html(raw).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// Extract href from `href="..."` at the given position in the string.
fn extract_href(haystack: &str) -> Option<String> {
    let key = "href=\"";
    let start = haystack.find(key)? + key.len();
    let rest = &haystack[start..];
    let end = rest.find('"')?;
    let url = &rest[..end];
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url.to_string())
    } else {
        None
    }
}

/// The three fixed service addresses; the test base replaces all three (DEC-007).
struct Endpoints {
    search: String,
    news: String,
    suggestions: String,
}

fn endpoints(config: &crate::config::WebConfig) -> Endpoints {
    #[cfg(test)]
    if let Some(base) = &config.test_base {
        return Endpoints {
            search: format!("{base}/html/"),
            news: format!("{base}/news.js"),
            suggestions: format!("{base}/ac/"),
        };
    }
    let _ = config;
    Endpoints {
        search: "https://html.duckduckgo.com/html/".to_string(),
        news: "https://duckduckgo.com/news.js".to_string(),
        suggestions: "https://duckduckgo.com/ac/".to_string(),
    }
}

async fn web_search(
    ep: &Endpoints,
    query: &str,
    max_results: usize,
    safe_search: &str,
) -> crate::errors::Result<Value> {
    let encoded = urlencoding(query);
    let kp = safe_param(safe_search);
    let url = format!("{}?q={encoded}&kp={kp}&kl=us-en", ep.search);

    let client = build_client(10)?;
    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            ToolError::internal("request timed out")
        } else {
            ToolError::internal(format!("search request failed: {e}"))
        }
    })?;

    let bytes = read_answer(resp, &url, ANSWER_LIMIT, "read response", |_| {
        over_answer_limit(&host_of(&url))
    })
    .await?;
    let html = String::from_utf8_lossy(&bytes);

    let mut results: Vec<Value> = Vec::new();

    // Split on result blocks.
    let parts: Vec<&str> = html.split("result__a").collect();
    let mut i = 1usize;
    while i < parts.len() && results.len() < max_results {
        let block = parts[i];
        // The previous split gave us just past "result__a"; walk backward to find href.
        // Actually after splitting on "result__a", the part before had `<a class="result__a" href="...">text`.
        // The href is in the tag attributes which come just before "result__a" in the original.
        // Let's search for href within the first few hundred chars of this block.
        let tag_region = if block.len() > 300 { &block[..300] } else { block };
        let url_opt = extract_href(tag_region);
        // The title text is between the closing `>` of the `<a>` tag and `</a>`.
        let title_opt = {
            let close = tag_region.find('>');
            if let Some(pos) = close {
                let after = &tag_region[pos + 1..];
                let end = after.find('<').unwrap_or(after.len());
                let t = strip_html(&after[..end]).trim().to_string();
                if t.is_empty() { None } else { Some(t) }
            } else {
                None
            }
        };

        // Snippet comes later in the block under result__snippet.
        let snippet_opt = extract_between(block, "result__snippet");

        if let (Some(url), Some(title)) = (url_opt, title_opt) {
            results.push(json!({
                "title": title,
                "url": url,
                "snippet": snippet_opt.unwrap_or_default(),
            }));
        }
        i += 1;
    }

    Ok(Value::String(serde_json::to_string_pretty(&results).unwrap_or_default()))
}

async fn web_news(
    ep: &Endpoints,
    query: &str,
    max_results: usize,
    time_range: Option<&str>,
) -> crate::errors::Result<Value> {
    let encoded = urlencoding(query);
    let df = match time_range {
        Some("d") => "d",
        Some("w") => "w",
        Some("m") => "m",
        Some("y") => "y",
        _ => "",
    };
    let url = if df.is_empty() {
        format!("{}?q={encoded}&o=json&noamp=1&kl=us-en", ep.news)
    } else {
        format!("{}?q={encoded}&o=json&noamp=1&kl=us-en&df={df}", ep.news)
    };

    let client = build_client(10)?;
    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            ToolError::internal("request timed out")
        } else {
            ToolError::internal(format!("news request failed: {e}"))
        }
    })?;

    let bytes = read_answer(resp, &url, ANSWER_LIMIT, "parse news response", |_| {
        over_answer_limit(&host_of(&url))
    })
    .await?;
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ToolError::internal("parse news response: error decoding response body"))?;

    let empty = vec![];
    let items = body["results"].as_array().unwrap_or(&empty);
    let results: Vec<Value> = items
        .iter()
        .take(max_results)
        .map(|item| {
            json!({
                "title": item["title"].as_str().unwrap_or(""),
                "url": item["url"].as_str().unwrap_or(""),
                "snippet": item["excerpt"].as_str().unwrap_or(""),
                "date": item["date"].as_str().unwrap_or(""),
            })
        })
        .collect();

    Ok(Value::String(serde_json::to_string_pretty(&results).unwrap_or_default()))
}

/// The most of one remote answer handed back to the assistant (equal to the pull request tools').
pub(crate) const ANSWER_LIMIT: u64 = 16_777_216;

/// Fetch a URL and return the raw bytes plus the Content-Type header value.
///
/// Used by both `web.fetch` (save mode) and `web.download`. The URL validation
/// (http/https prefix) is the caller's responsibility.
async fn fetch_raw(
    url: &str,
    timeout_secs: u64,
    cap: u64,
    over: impl Fn(Option<u64>) -> ToolError,
) -> crate::errors::Result<(Vec<u8>, String)> {
    let client = build_client(timeout_secs)?;
    let resp = client.get(url).send().await.map_err(|e| {
        if e.is_timeout() {
            ToolError::internal("request timed out")
        } else {
            ToolError::internal(format!("fetch request failed: {e}"))
        }
    })?;
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = read_answer(resp, url, cap, "read response", over).await?;
    Ok((bytes, content_type))
}

/// Save mode and download read the whole answer until the write limit story: no cap, no refusal.
fn unreachable_over(_declared: Option<u64>) -> ToolError {
    ToolError::internal("answer limit reached with no limit set")
}

/// The lower case host of `url`, no port, read before sending so a redirect never changes it.
pub(crate) fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_default()
}

/// The refusal of an answer past [`ANSWER_LIMIT`], worded as the pull request tools word theirs.
pub(crate) fn over_answer_limit(host: &str) -> ToolError {
    ToolError::invalid_argument(format!("response from host '{host}' exceeds the 16 MiB limit"))
}

/// Read a remote answer piece by piece, refusing once it passes `cap`.
///
/// A declared length past `cap` is refused before any body byte is read. While
/// reading, the size is checked before each piece is appended, so at most `cap`
/// bytes plus one piece are ever held. The response is dropped on refusal, which
/// closes the connection without waiting for the site. No decompression is
/// configured on the client, so the bytes counted are the bytes kept.
///
/// Time O(n) in the bytes read, space O(min(n, cap)) plus one piece.
pub(crate) async fn read_answer(
    mut resp: reqwest::Response,
    url: &str,
    cap: u64,
    read_err: &str,
    over: impl Fn(Option<u64>) -> ToolError,
) -> crate::errors::Result<Vec<u8>> {
    let held = start_answer(url);
    if let Some(n) = resp.content_length()
        && n > cap
    {
        return Err(over(Some(n)));
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(piece) =
        resp.chunk().await.map_err(|e| ToolError::internal(format!("{read_err}: {e}")))?
    {
        let total = buf.len() as u64 + piece.len() as u64;
        held.note(total);
        if total > cap {
            return Err(over(None));
        }
        buf.extend_from_slice(&piece);
    }
    Ok(buf)
}

async fn web_fetch(url: &str, timeout_secs: u64) -> crate::errors::Result<Value> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(ToolError::invalid_argument("url must start with http:// or https://"));
    }

    let (bytes, content_type) =
        fetch_raw(url, timeout_secs, ANSWER_LIMIT, |_| over_answer_limit(&host_of(url))).await?;
    let body = String::from_utf8_lossy(&bytes).into_owned();

    let text = if content_type.contains("text/html") { strip_html(&body) } else { body };

    let capped: String = text.chars().take(50_000).collect();
    Ok(Value::String(capped))
}

async fn web_suggestions(ep: &Endpoints, query: &str) -> crate::errors::Result<Value> {
    let encoded = urlencoding(query);
    let url = format!("{}?q={encoded}&type=list", ep.suggestions);

    let client = build_client(10)?;
    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            ToolError::internal("request timed out")
        } else {
            ToolError::internal(format!("suggestions request failed: {e}"))
        }
    })?;

    let bytes = read_answer(resp, &url, ANSWER_LIMIT, "parse suggestions response", |_| {
        over_answer_limit(&host_of(&url))
    })
    .await?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| {
        ToolError::internal("parse suggestions response: error decoding response body")
    })?;

    let suggestions = body.get(1).cloned().unwrap_or(Value::Array(vec![]));
    Ok(Value::String(serde_json::to_string_pretty(&suggestions).unwrap_or_default()))
}

/// Percent-encode a query string for use in URLs.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// What a test reads back of one answer read (DEC-003).
#[cfg(test)]
pub(crate) struct AnswerHeld {
    pub(crate) url: String,
    pub(crate) held: u64,
}

#[cfg(test)]
thread_local! {
    pub(crate) static ANSWER_HELD: std::cell::RefCell<Vec<AnswerHeld>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Test only record of the most a call held of one remote answer. It is pushed
/// at once with 0 and written in `Drop`, so every return path leaves one record.
#[cfg(test)]
pub(crate) struct HeldRecord {
    index: usize,
    held: std::cell::Cell<u64>,
}

#[cfg(test)]
impl HeldRecord {
    fn note(&self, held: u64) {
        self.held.set(self.held.get().max(held));
    }
}

#[cfg(test)]
impl Drop for HeldRecord {
    fn drop(&mut self) {
        ANSWER_HELD.with(|h| {
            if let Some(r) = h.borrow_mut().get_mut(self.index) {
                r.held = self.held.get();
            }
        });
    }
}

#[cfg(test)]
fn start_answer(url: &str) -> HeldRecord {
    let index = ANSWER_HELD.with(|h| {
        let mut h = h.borrow_mut();
        h.push(AnswerHeld { url: url.to_string(), held: 0 });
        h.len() - 1
    });
    HeldRecord { index, held: std::cell::Cell::new(0) }
}

/// Release build twin of the test hook: nothing recorded, nothing kept.
#[cfg(not(test))]
pub(crate) struct HeldRecord;

#[cfg(not(test))]
impl HeldRecord {
    fn note(&self, _held: u64) {}
}

#[cfg(not(test))]
fn start_answer(_url: &str) -> HeldRecord {
    HeldRecord
}

#[cfg(test)]
pub(crate) fn answer_held_reset() {
    ANSWER_HELD.with(|h| h.borrow_mut().clear());
}

/// Every held figure recorded for `url`; panics when none, so a missing record never reads as 0.
#[cfg(test)]
pub(crate) fn answer_held(url: &str) -> Vec<u64> {
    let v: Vec<u64> =
        ANSWER_HELD.with(|h| h.borrow().iter().filter(|r| r.url == url).map(|r| r.held).collect());
    assert!(!v.is_empty(), "no answer was read for {url}");
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WebConfig;
    use crate::tools::registry_support::ToolRegistry;

    fn reg() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        register(&mut r, &WebConfig::default());
        r
    }

    #[test]
    fn four_web_tools_register() {
        assert_eq!(reg().len(), 5);
        assert_eq!(
            reg().names(),
            ["web.search", "web.news", "web.fetch", "web.download", "web.suggestions"]
        );
    }

    #[test]
    fn web_search_schema_has_required_query() {
        let r = reg();
        let t = r.resolve("web.search").unwrap();
        let schema = t.schema.input_schema();
        let req = schema["required"].as_array().unwrap();
        assert_eq!(req, &[serde_json::json!("query")]);
    }

    #[test]
    fn strip_html_removes_tags_and_decodes_entities() {
        assert_eq!(strip_html("<b>Hello</b> &amp; world"), "Hello & world");
        assert_eq!(strip_html("a<br/>b"), "ab");
        assert_eq!(strip_html("&lt;tag&gt;"), "<tag>");
        assert_eq!(strip_html("&quot;quoted&quot;"), "\"quoted\"");
        assert_eq!(strip_html("&nbsp;space"), " space");
    }

    #[test]
    fn safe_search_param_maps_correctly() {
        assert_eq!(safe_param("strict"), "1");
        assert_eq!(safe_param("off"), "-1");
        assert_eq!(safe_param("moderate"), "0");
        assert_eq!(safe_param("anything"), "0");
    }

    #[test]
    fn max_results_cap_is_enforced() {
        assert_eq!(cap_results(5, 10), 5);
        assert_eq!(cap_results(100, 10), 50);
        assert_eq!(cap_results(0, 10), 10);
    }
}
