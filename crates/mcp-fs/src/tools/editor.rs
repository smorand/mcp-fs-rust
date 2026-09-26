//! `doc.open_editor`, `doc.close_editor`, `doc.list_editors`
//!
//! Each `doc.open_editor` call spawns a lightweight axum server on an
//! OS-assigned port. The server:
//!   GET /    → returns the full editor shell page (HTML + CSS + JS, all inline)
//!   GET /ws  → WebSocket: pushes `{"type":"reload","html":"..."}` when the volume
//!              file changes externally; receives `{"type":"save","html":"..."}` to
//!              write back to the volume.
//!
//! A tokio task polls the file's `mtime` every 500 ms and broadcasts a reload
//! message to all connected WebSocket clients when it detects a change.
//!
//! The editor lives until `doc.close_editor` is called or the server shuts down.
//! Opening the same path a second time returns the existing editor (idempotent).
//!
//! The browser system command (`open` on macOS, `xdg-open` on Linux) is called
//! automatically after the server is ready, except during tests.

use crate::errors::{Result, ToolError};
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::storage::VolumeClient;
use axum::Router;
use axum::extract::State as AxumState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, broadcast};

// ── public registry ───────────────────────────────────────────────────────────

pub struct EditorRegistry {
    editors: Mutex<HashMap<String, EditorSlot>>,
}

/// What we keep per open editor.
struct EditorSlot {
    editor_id: String,
    mount: String,
    path: String,
    mode: EditorMode,
    port: u16,
    /// Sending a value shuts the server task down.
    _shutdown_tx: tokio::sync::oneshot::Sender<()>,
    /// Broadcast channel to push reload messages to all connected WebSockets.
    _reload_tx: broadcast::Sender<String>,
    /// Set to false when the slot is closed so background tasks can stop.
    alive: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditorMode {
    Doc,
    Slides,
}

impl EditorMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Doc => "doc",
            Self::Slides => "slides",
        }
    }
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "doc" => Ok(Self::Doc),
            "slides" => Ok(Self::Slides),
            other => Err(ToolError::invalid_argument(format!(
                "mode must be 'doc' or 'slides', got '{other}'"
            ))),
        }
    }
}

impl Default for EditorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorRegistry {
    pub fn new() -> Self {
        Self { editors: Mutex::new(HashMap::new()) }
    }

    /// Close all editors. Called on server shutdown.
    pub async fn close_all(&self) {
        let mut map = self.editors.lock().await;
        for slot in map.values() {
            slot.alive.store(false, Ordering::Relaxed);
        }
        map.drain();
    }
}

// ── tool registration ─────────────────────────────────────────────────────────

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new(
            "doc.open_editor",
            "Open an interactive HTML editor for a file in the volume. Starts a local HTTP \
             server with live sync: edits in the browser are saved back to the volume, and \
             external changes (e.g. via fs.write) are pushed to the browser. Returns the \
             editor URL and a unique editor_id.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Absolute POSIX path of the HTML file to edit.")
        .req_str(
            "mode",
            "Editor mode: 'doc' for a document (CMS-like) or 'slides' for a slide deck.",
        )
        .destructive(false)
        .read_only(false)
        .idempotent(false)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = a.str("mount_id")?;
            ctx.state.authorize(&mount, &ctx.person).await?;
            let path = ctx.state.safety.normalize_path(&a.str("path")?)?;
            let mode = EditorMode::from_str(&a.str("mode")?)?;
            open_editor(
                &ctx.state.editors,
                &ctx.state.stores.client(&mount).await?,
                &mount,
                &path,
                mode,
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new(
            "doc.close_editor",
            "Stop the editor server for the given editor_id and release its port.",
        )
        .req_str("editor_id", "ID returned by doc.open_editor.")
        .destructive(false)
        .read_only(false)
        .idempotent(true)
        .open_world(false),
        handler(|ctx, a| async move {
            let id = a.str("editor_id")?;
            let mut map = ctx.state.editors.editors.lock().await;
            if let Some(slot) = map.remove(&id) {
                slot.alive.store(false, Ordering::Relaxed);
                Ok(json!({ "editor_id": id, "closed": true }))
            } else {
                Err(ToolError::not_found(format!("no active editor with id '{id}'")))
            }
        }),
    );

    reg.add(
        ToolSchema::new("doc.list_editors", "List all active HTML editors and their URLs.")
            .read_only(true)
            .idempotent(true)
            .open_world(false),
        handler(|ctx, _a| async move {
            let map = ctx.state.editors.editors.lock().await;
            let editors: Vec<_> = map
                .values()
                .map(|s| {
                    json!({
                        "editor_id": s.editor_id,
                        "url": format!("http://127.0.0.1:{}", s.port),
                        "path": s.path,
                        "mode": s.mode.as_str(),
                    })
                })
                .collect();
            Ok(json!({ "editors": editors }))
        }),
    );
}

// ── open_editor core ──────────────────────────────────────────────────────────

async fn open_editor(
    registry: &Arc<EditorRegistry>,
    client: &Arc<VolumeClient>,
    mount: &str,
    path: &str,
    mode: EditorMode,
) -> Result<serde_json::Value> {
    // Idempotent: return existing editor for same (mount, path).
    {
        let map = registry.editors.lock().await;
        for slot in map.values() {
            if slot.mount == mount && slot.path == path {
                return Ok(json!({
                    "editor_id": slot.editor_id,
                    "url": format!("http://127.0.0.1:{}", slot.port),
                    "path": path,
                    "mode": slot.mode.as_str(),
                    "created": false,
                }));
            }
        }
    }

    // Create file if missing.
    let created = !client.exists(path).await.unwrap_or(false);
    if created {
        let html = starter_html(mode, path);
        client
            .write_text_atomic(path, &html)
            .await
            .map_err(|e| ToolError::internal(format!("cannot create file: {e}")))?;
    }

    // Bind on an OS-assigned port.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| ToolError::internal(format!("cannot bind port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| ToolError::internal(format!("cannot get port: {e}")))?
        .port();

    let editor_id = uuid::Uuid::new_v4().to_string();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let (reload_tx, _) = broadcast::channel::<String>(64);
    let alive = Arc::new(AtomicBool::new(true));

    // Shared state for the mini-server.
    let server_state = Arc::new(MiniServerState {
        client: client.clone(),
        path: path.to_string(),
        mode,
        reload_tx: reload_tx.clone(),
    });

    let app = Router::new()
        .route("/", get(serve_editor_page))
        .route("/ws", get(ws_handler))
        .with_state(server_state.clone());

    // Spawn the server task.
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    // Spawn the file watcher task.
    let watcher_client = client.clone();
    let watcher_path = path.to_string();
    let watcher_reload_tx = reload_tx.clone();
    let watcher_alive = alive.clone();
    tokio::spawn(async move {
        let mut last_mtime = 0f64;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if !watcher_alive.load(Ordering::Relaxed) {
                break;
            }
            if let Ok(stat) = watcher_client.stat(&watcher_path).await {
                if stat.mtime > last_mtime
                    && last_mtime > 0.0
                    && let Ok(bytes) = watcher_client.read_bytes(&watcher_path).await
                {
                    let full_html = String::from_utf8_lossy(&bytes).to_string();
                    let content = extract_content(&full_html).unwrap_or_default();
                    let msg = serde_json::json!({
                        "type": "reload",
                        "html": content,
                    })
                    .to_string();
                    // Ignore send error: no clients connected is fine.
                    let _ = watcher_reload_tx.send(msg);
                }
                last_mtime = stat.mtime;
            }
        }
    });

    // Store the slot.
    {
        let mut map = registry.editors.lock().await;
        map.insert(
            editor_id.clone(),
            EditorSlot {
                editor_id: editor_id.clone(),
                mount: mount.to_string(),
                path: path.to_string(),
                mode,
                port,
                _shutdown_tx: shutdown_tx,
                _reload_tx: reload_tx,
                alive,
            },
        );
    }

    let url = format!("http://127.0.0.1:{port}");

    // Open the browser (skipped in tests).
    open_browser(&url);

    Ok(json!({
        "editor_id": editor_id,
        "url": url,
        "path": path,
        "mode": mode.as_str(),
        "created": created,
    }))
}

// ── mini-server state & handlers ─────────────────────────────────────────────

#[derive(Clone)]
struct MiniServerState {
    client: Arc<VolumeClient>,
    path: String,
    mode: EditorMode,
    reload_tx: broadcast::Sender<String>,
}

async fn serve_editor_page(AxumState(s): AxumState<Arc<MiniServerState>>) -> impl IntoResponse {
    let content = match s.client.read_text(&s.path).await {
        Ok(html) => extract_content(&html).unwrap_or_else(|| "<p></p>".to_string()),
        Err(_) => "<p></p>".to_string(),
    };
    let html = build_editor_shell(&content, s.mode);
    axum::response::Html(html)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    AxumState(s): AxumState<Arc<MiniServerState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, s))
}

async fn handle_ws(mut socket: WebSocket, s: Arc<MiniServerState>) {
    let mut reload_rx = s.reload_tx.subscribe();
    loop {
        tokio::select! {
            // Outbound: forward reload messages from the watcher.
            msg = reload_rx.recv() => {
                match msg {
                    Ok(text) => {
                        if socket.send(Message::Text(text.into())).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
            // Inbound: receive save messages from the browser.
            frame = socket.recv() => {
                match frame {
                    None => break,
                    Some(Err(_)) => break,
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Text(text))) => {
                        handle_ws_message(&s, text.as_str()).await;
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn handle_ws_message(s: &MiniServerState, text: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    if v["type"].as_str() != Some("save") {
        return;
    }
    let Some(new_content) = v["html"].as_str() else {
        return;
    };

    // Read current file, replace #content, write back.
    let Ok(current) = s.client.read_text(&s.path).await else {
        return;
    };
    let updated = inject_content(&current, new_content);
    let _ = s.client.write_text_atomic(&s.path, &updated).await;
}

// ── HTML helpers ──────────────────────────────────────────────────────────────

/// Extract everything between `<div id="content">` and its closing `</div>`.
///
/// The match is on the literal marker strings so no HTML parser is needed.
/// Works because the templates we write always have the markers on their own
/// content boundary. Returns `None` if the markers are not found.
pub fn extract_content(html: &str) -> Option<String> {
    let open_tag = "<div id=\"content\">";
    let start = html.find(open_tag)? + open_tag.len();
    // Find the matching close tag by tracking nesting depth.
    let tail = &html[start..];
    let mut depth: usize = 1;
    let mut pos = 0;
    while pos < tail.len() {
        if tail[pos..].starts_with("<div") {
            depth += 1;
            pos += 4;
        } else if tail[pos..].starts_with("</div>") {
            depth -= 1;
            if depth == 0 {
                return Some(tail[..pos].to_string());
            }
            pos += 6;
        } else {
            pos += 1;
        }
    }
    None
}

/// Replace the content of `<div id="content">...</div>` in `html`.
///
/// If the markers are not found the document is returned unchanged.
pub fn inject_content(html: &str, new_content: &str) -> String {
    let Some(old_content) = extract_content(html) else {
        return html.to_string();
    };
    let open_tag = "<div id=\"content\">";
    let start = html.find(open_tag).unwrap() + open_tag.len();
    let before = &html[..start];
    let after = &html[start + old_content.len()..];
    format!("{before}{new_content}{after}")
}

fn starter_html(mode: EditorMode, _path: &str) -> String {
    let content = match mode {
        EditorMode::Doc => "    <h1>Title</h1>\n    <p>Start writing here.</p>",
        EditorMode::Slides => "    <section><h1>Slide 1</h1><p>Content.</p></section>",
    };
    format!(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"><title>Document</title></head>\n\
         <body>\n<div id=\"content\">\n{content}\n</div>\n</body>\n</html>\n"
    )
}

/// Build the full editor shell page. All CSS and JS are inline.
fn build_editor_shell(content: &str, mode: EditorMode) -> String {
    let slide_nav = if mode == EditorMode::Slides {
        r#"<nav id="slide-nav" style="position:fixed;top:0;left:0;right:0;background:#222;color:#fff;padding:6px 12px;display:flex;align-items:center;gap:12px;z-index:100;font-family:sans-serif;font-size:14px">
  <button id="prev" style="padding:2px 10px;cursor:pointer">◀</button>
  <span id="slide-counter">1 / 1</span>
  <button id="next" style="padding:2px 10px;cursor:pointer">▶</button>
  <span id="sync-indicator" style="margin-left:auto;opacity:0.6"></span>
</nav>
<div style="height:36px"></div>"#
    } else {
        r#"<div id="toolbar" style="position:fixed;top:0;right:12px;font-family:sans-serif;font-size:12px;color:#888;padding:4px 0;z-index:100">
  <span id="sync-indicator"></span>
</div>"#
    };

    let body_style = match mode {
        EditorMode::Doc => {
            "margin:40px auto;max-width:780px;font-family:Georgia,serif;line-height:1.7;padding:0 24px"
        }
        EditorMode::Slides => "margin:0;overflow:hidden",
    };

    let content_style = match mode {
        EditorMode::Doc => "outline:none;min-height:200px",
        EditorMode::Slides => "height:calc(100vh - 36px);overflow:hidden",
    };

    let slide_js = if mode == EditorMode::Slides {
        r#"
    // Slide navigation
    let currentSlide = 0;
    function slides() { return Array.from(content.querySelectorAll('section')); }
    function showSlide(n) {
      const ss = slides();
      if (!ss.length) return;
      currentSlide = Math.max(0, Math.min(n, ss.length - 1));
      ss.forEach((s, i) => s.style.display = i === currentSlide ? 'block' : 'none');
      document.getElementById('slide-counter').textContent = (currentSlide+1) + ' / ' + ss.length;
    }
    document.getElementById('prev').addEventListener('click', () => showSlide(currentSlide - 1));
    document.getElementById('next').addEventListener('click', () => showSlide(currentSlide + 1));
    showSlide(0);
    function refreshSlideNav() { showSlide(currentSlide); }
"#
    } else {
        ""
    };

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>Editor</title>
<style>
* {{ box-sizing: border-box; }}
body {{ {body_style} }}
#content {{ {content_style} }}
#content section {{ height:calc(100vh - 36px);display:flex;flex-direction:column;justify-content:center;padding:60px;font-family:sans-serif }}
[contenteditable]:focus {{ outline:none }}
</style>
</head>
<body>
{slide_nav}
<div id="content" contenteditable="true">{content}</div>
<script>
(function() {{
  const content = document.getElementById('content');
  const indicator = document.getElementById('sync-indicator');
  {slide_js}

  // WebSocket
  let ws, debounceTimer;
  function connect() {{
    ws = new WebSocket('ws://' + location.host + '/ws');
    ws.onmessage = function(e) {{
      const msg = JSON.parse(e.data);
      if (msg.type === 'reload') {{
        content.innerHTML = msg.html;
        indicator.textContent = '↺ reloaded';
        setTimeout(() => indicator.textContent = '', 2000);
        {slide_js_refresh}
      }}
    }};
    ws.onclose = function() {{ setTimeout(connect, 1500); }};
  }}
  connect();

  // Save on input (debounced 800ms)
  content.addEventListener('input', function() {{
    clearTimeout(debounceTimer);
    indicator.textContent = '…';
    debounceTimer = setTimeout(function() {{
      if (ws && ws.readyState === 1) {{
        ws.send(JSON.stringify({{ type: 'save', html: content.innerHTML }}));
        indicator.textContent = '✓ saved';
        setTimeout(() => indicator.textContent = '', 2000);
      }}
    }}, 800);
  }});
}})();
</script>
</body>
</html>
"#,
        slide_js_refresh = if mode == EditorMode::Slides {
            "if(typeof refreshSlideNav==='function') refreshSlideNav();"
        } else {
            ""
        },
    )
}

// ── browser opener ────────────────────────────────────────────────────────────

fn open_browser(url: &str) {
    #[cfg(not(test))]
    {
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(url).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let _ = url; // no-op on other platforms
    }
    #[cfg(test)]
    let _ = url;
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testkit::{MOUNT, assert_family, assert_schema, harness_with_extra};
    use serde_json::json;

    fn register_editor(reg: &mut ToolRegistry, _cfg: &crate::config::ServerConfig) {
        register(reg);
    }

    // ── schema / family ────────────────────────────────────────────────────────

    #[test]
    fn family_registers_three_editor_tools() {
        assert_family(register, &["doc.open_editor", "doc.close_editor", "doc.list_editors"]);
    }

    #[test]
    fn doc_open_editor_schema() {
        assert_schema(
            register,
            "doc.open_editor",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path of the HTML file to edit.","type":"string"},
                 "mode":{"description":"Editor mode: 'doc' for a document (CMS-like) or 'slides' for a slide deck.","type":"string"}},
               "required":["mount_id","path","mode"]}"#,
        );
    }

    #[test]
    fn doc_close_editor_schema() {
        assert_schema(
            register,
            "doc.close_editor",
            r#"{"type":"object","properties":{
                 "editor_id":{"description":"ID returned by doc.open_editor.","type":"string"}},
               "required":["editor_id"]}"#,
        );
    }

    #[test]
    fn doc_list_editors_schema() {
        assert_schema(register, "doc.list_editors", r#"{"type":"object","properties":{}}"#);
    }

    // ── open / close / list ────────────────────────────────────────────────────

    #[tokio::test]
    async fn editor_creates_file_if_missing_doc() {
        let h = harness_with_extra(register_editor).await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/new.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        assert_eq!(r["created"], true);
        assert!(r["url"].as_str().unwrap().starts_with("http://127.0.0.1:"));
        let html = h.client().await.read_text("/new.html").await.unwrap();
        assert!(html.contains(r#"<div id="content">"#));
        // clean up
        let id = r["editor_id"].as_str().unwrap().to_string();
        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    #[tokio::test]
    async fn editor_creates_file_if_missing_slides() {
        let h = harness_with_extra(register_editor).await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/deck.html", "mode": "slides"
                }),
            )
            .await
            .unwrap();
        assert_eq!(r["created"], true);
        let html = h.client().await.read_text("/deck.html").await.unwrap();
        assert!(html.contains("<section>"));
        let id = r["editor_id"].as_str().unwrap().to_string();
        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    #[tokio::test]
    async fn editor_open_returns_url_and_id() {
        let h = harness_with_extra(register_editor).await;
        h.seed(
            "/doc.html",
            r#"<!DOCTYPE html><html><body><div id="content"><p>hi</p></div></body></html>"#,
        )
        .await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/doc.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        assert_eq!(r["created"], false);
        assert!(!r["editor_id"].as_str().unwrap().is_empty());
        assert!(r["url"].as_str().unwrap().starts_with("http://127.0.0.1:"));
        let id = r["editor_id"].as_str().unwrap().to_string();
        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    #[tokio::test]
    async fn editor_list_shows_open_editor() {
        let h = harness_with_extra(register_editor).await;
        h.seed("/a.html", r#"<html><body><div id="content"><p>x</p></div></body></html>"#).await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/a.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        let id = r["editor_id"].as_str().unwrap().to_string();
        let list = h.call("doc.list_editors", json!({})).await.unwrap();
        let editors = list["editors"].as_array().unwrap();
        assert!(editors.iter().any(|e| e["editor_id"] == id));
        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    #[tokio::test]
    async fn editor_close_removes_from_list() {
        let h = harness_with_extra(register_editor).await;
        h.seed("/b.html", r#"<html><body><div id="content"><p>x</p></div></body></html>"#).await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/b.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        let id = r["editor_id"].as_str().unwrap().to_string();
        h.call("doc.close_editor", json!({"editor_id": id.clone()})).await.unwrap();
        let list = h.call("doc.list_editors", json!({})).await.unwrap();
        let editors = list["editors"].as_array().unwrap();
        assert!(!editors.iter().any(|e| e["editor_id"] == id));
    }

    #[tokio::test]
    async fn editor_second_open_same_path_returns_existing() {
        let h = harness_with_extra(register_editor).await;
        h.seed("/c.html", r#"<html><body><div id="content"><p>x</p></div></body></html>"#).await;
        let r1 = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/c.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        let r2 = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/c.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        assert_eq!(r1["editor_id"], r2["editor_id"], "second open must return same id");
        assert_eq!(r2["created"], false);
        let id = r1["editor_id"].as_str().unwrap().to_string();
        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    // ── HTTP / WebSocket integration ───────────────────────────────────────────

    #[tokio::test]
    async fn editor_get_serves_html() {
        let h = harness_with_extra(register_editor).await;
        h.seed(
            "/page.html",
            r#"<!DOCTYPE html><html><body><div id="content"><p>hello</p></div></body></html>"#,
        )
        .await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/page.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        let url = r["url"].as_str().unwrap().to_string();
        let id = r["editor_id"].as_str().unwrap().to_string();

        // Give the server a moment to start.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
        assert!(body.contains(r#"<div id="content"#), "shell must contain #content");
        assert!(body.contains("hello"), "must inject file content");

        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    #[tokio::test]
    async fn editor_ws_save_writes_to_volume() {
        use futures::SinkExt;
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message as TMsg;

        let h = harness_with_extra(register_editor).await;
        h.seed(
            "/ws_test.html",
            r#"<!DOCTYPE html><html><body><div id="content"><p>original</p></div></body></html>"#,
        )
        .await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/ws_test.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        let port = {
            let url = r["url"].as_str().unwrap();
            url.split(':').next_back().unwrap().parse::<u16>().unwrap()
        };
        let id = r["editor_id"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ws_url = format!("ws://127.0.0.1:{port}/ws");
        let (mut ws, _) = connect_async(&ws_url).await.unwrap();

        let save_msg = json!({"type": "save", "html": "<p>updated by ws</p>"}).to_string();
        ws.send(TMsg::Text(save_msg.into())).await.unwrap();

        // Give the handler time to write.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let html = h.client().await.read_text("/ws_test.html").await.unwrap();
        assert!(html.contains("updated by ws"), "volume must reflect the WS save");

        ws.close(None).await.ok();
        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    #[tokio::test]
    async fn editor_watcher_pushes_reload() {
        use futures::StreamExt;
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message as TMsg;

        let h = harness_with_extra(register_editor).await;
        h.seed(
            "/watch_test.html",
            r#"<!DOCTYPE html><html><body><div id="content"><p>v1</p></div></body></html>"#,
        )
        .await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/watch_test.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        let port = {
            let url = r["url"].as_str().unwrap();
            url.split(':').next_back().unwrap().parse::<u16>().unwrap()
        };
        let id = r["editor_id"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;

        let ws_url = format!("ws://127.0.0.1:{port}/ws");
        let (mut ws, _) = connect_async(&ws_url).await.unwrap();

        // Wait for the watcher to record the initial mtime (one poll cycle).
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;

        // Modify the file externally (simulating fs.write).
        h.client()
            .await
            .write_text_atomic(
                "/watch_test.html",
                r#"<!DOCTYPE html><html><body><div id="content"><p>v2</p></div></body></html>"#,
            )
            .await
            .unwrap();

        // Wait up to 1.5 s for the reload message.
        let reload_msg = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match ws.next().await {
                    Some(Ok(TMsg::Text(t))) => {
                        let v: serde_json::Value =
                            serde_json::from_str(t.as_str()).unwrap_or_default();
                        if v["type"] == "reload" {
                            return v;
                        }
                    }
                    _ => continue,
                }
            }
        })
        .await
        .expect("expected a reload message within 2s");

        assert_eq!(reload_msg["type"], "reload");
        assert!(reload_msg["html"].as_str().unwrap().contains("v2"));

        ws.close(None).await.ok();
        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    // ── extract / inject ───────────────────────────────────────────────────────

    #[test]
    fn extract_content_roundtrip() {
        let html = r#"<html><body><div id="content"><p>hello</p></div></body></html>"#;
        let extracted = extract_content(html).unwrap();
        assert_eq!(extracted, "<p>hello</p>");
        let injected = inject_content(html, "<p>world</p>");
        assert_eq!(extract_content(&injected).unwrap(), "<p>world</p>");
    }

    #[test]
    fn extract_content_handles_nested_divs() {
        let html = r#"<body><div id="content"><div class="a"><div>x</div></div></div></body>"#;
        let c = extract_content(html).unwrap();
        assert_eq!(c, r#"<div class="a"><div>x</div></div>"#);
    }

    #[test]
    fn extract_content_returns_none_when_marker_absent() {
        assert!(extract_content("<html><body><p>hi</p></body></html>").is_none());
    }

    #[test]
    fn inject_content_leaves_html_unchanged_when_marker_absent() {
        let html = "<html><body><p>x</p></body></html>";
        assert_eq!(inject_content(html, "new"), html);
    }

    // ── new tests ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn open_editor_invalid_mode_returns_err_invalid_argument() {
        let h = harness_with_extra(register_editor).await;
        let err = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/doc.html", "mode": "pdf"
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("mode must be"), "expected mode error, got: {}", err.message);
    }

    #[tokio::test]
    async fn close_editor_unknown_id_returns_err_not_found() {
        let h = harness_with_extra(register_editor).await;
        let random_id = uuid::Uuid::new_v4().to_string();
        let err = h.call("doc.close_editor", json!({"editor_id": random_id})).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_editors_returns_empty_array_when_none_open() {
        let h = harness_with_extra(register_editor).await;
        let r = h.call("doc.list_editors", json!({})).await.unwrap();
        assert_eq!(r["editors"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn ws_unknown_message_type_is_silently_ignored() {
        use futures::SinkExt;
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message as TMsg;

        let h = harness_with_extra(register_editor).await;
        h.seed(
            "/ping_test.html",
            r#"<!DOCTYPE html><html><body><div id="content"><p>original</p></div></body></html>"#,
        )
        .await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/ping_test.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        let port = {
            let url = r["url"].as_str().unwrap();
            url.split(':').next_back().unwrap().parse::<u16>().unwrap()
        };
        let id = r["editor_id"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ws_url = format!("ws://127.0.0.1:{port}/ws");
        let (mut ws, _) = connect_async(&ws_url).await.unwrap();

        // Send an unknown message type; the server must not crash or write anything.
        let ping_msg = json!({"type": "ping"}).to_string();
        ws.send(TMsg::Text(ping_msg.into())).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let content = h.client().await.read_text("/ping_test.html").await.unwrap();
        assert!(content.contains("original"), "file must be unchanged after unknown message type");

        ws.close(None).await.ok();
        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    #[tokio::test]
    async fn ws_save_without_content_marker_is_a_noop() {
        use futures::SinkExt;
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message as TMsg;

        let h = harness_with_extra(register_editor).await;
        // File has no <div id="content"> marker.
        h.seed("/no_marker.html", "<html><body><p>no marker here</p></body></html>").await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/no_marker.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        let port = {
            let url = r["url"].as_str().unwrap();
            url.split(':').next_back().unwrap().parse::<u16>().unwrap()
        };
        let id = r["editor_id"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ws_url = format!("ws://127.0.0.1:{port}/ws");
        let (mut ws, _) = connect_async(&ws_url).await.unwrap();

        let save_msg = json!({"type": "save", "html": "<p>injected</p>"}).to_string();
        ws.send(TMsg::Text(save_msg.into())).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // inject_content returns original when marker is absent, so file must be unchanged.
        let content = h.client().await.read_text("/no_marker.html").await.unwrap();
        assert!(content.contains("no marker here"), "file must be unchanged when no marker");
        assert!(!content.contains("injected"), "injected content must not appear");

        ws.close(None).await.ok();
        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    #[tokio::test]
    async fn multiple_ws_clients_all_receive_reload() {
        use futures::StreamExt;
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message as TMsg;

        let h = harness_with_extra(register_editor).await;
        h.seed(
            "/multi_ws.html",
            r#"<!DOCTYPE html><html><body><div id="content"><p>v1</p></div></body></html>"#,
        )
        .await;
        let r = h
            .call(
                "doc.open_editor",
                json!({
                    "mount_id": MOUNT, "path": "/multi_ws.html", "mode": "doc"
                }),
            )
            .await
            .unwrap();
        let port = {
            let url = r["url"].as_str().unwrap();
            url.split(':').next_back().unwrap().parse::<u16>().unwrap()
        };
        let id = r["editor_id"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;

        let ws_url = format!("ws://127.0.0.1:{port}/ws");
        let (ws1, _) = connect_async(&ws_url).await.unwrap();
        let (ws2, _) = connect_async(&ws_url).await.unwrap();

        // Wait one poll cycle so the watcher records the initial mtime.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;

        // External write to trigger the watcher.
        h.client()
            .await
            .write_text_atomic(
                "/multi_ws.html",
                r#"<!DOCTYPE html><html><body><div id="content"><p>v2</p></div></body></html>"#,
            )
            .await
            .unwrap();

        // Both clients should get a reload message within 2 s.
        let recv_reload = |mut ws: tokio_tungstenite::WebSocketStream<_>| async move {
            tokio::time::timeout(std::time::Duration::from_secs(2), async move {
                loop {
                    match ws.next().await {
                        Some(Ok(TMsg::Text(t))) => {
                            let v: serde_json::Value =
                                serde_json::from_str(t.as_str()).unwrap_or_default();
                            if v["type"] == "reload" {
                                return v;
                            }
                        }
                        _ => continue,
                    }
                }
            })
            .await
            .expect("expected reload message within 2s")
        };

        let (m1, m2) = tokio::join!(recv_reload(ws1), recv_reload(ws2));
        assert_eq!(m1["type"], "reload");
        assert_eq!(m2["type"], "reload");
        assert!(m1["html"].as_str().unwrap().contains("v2"));
        assert!(m2["html"].as_str().unwrap().contains("v2"));

        h.call("doc.close_editor", json!({"editor_id": id})).await.unwrap();
    }

    #[test]
    fn inject_content_with_empty_new_content() {
        let html = r#"<html><body><div id="content"><p>original</p></div></body></html>"#;
        let result = inject_content(html, "");
        let extracted = extract_content(&result);
        assert_eq!(
            extracted,
            Some("".to_string()),
            "extract after inject-empty must return Some empty string"
        );
    }
}
