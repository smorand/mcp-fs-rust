//! `doc.*` tools: document conversion via pandoc.
//!
//! The family is optional: `register` is a no-op when pandoc is not found in PATH (or at
//! the configured bin path). A warning is logged so the operator knows why the tool is
//! absent.

use crate::config::DocConfig;
use crate::errors::{Result, ToolError};
use crate::mcp::registry::{ToolRegistry, handler};
use crate::mcp::ToolSchema;
use crate::safety::SafetyManager;
use crate::storage::VolumeClient;
use crate::tools::{norm, volume};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const DESCRIPTION: &str = "Convert a Markdown or HTML file in the volume to a .docx Word \
    document using pandoc. Supply a .docx template via template_path to apply custom \
    styles, headers, and footers.";

/// Pandoc binary path + timeout bundled to stay within clippy's argument limit.
struct PandocCtx {
    bin: String,
    timeout_secs: u64,
}

/// Attempt to resolve the pandoc binary path from the config or from PATH.
fn resolve_pandoc(config: &DocConfig) -> Option<String> {
    if !config.pandoc_bin.is_empty() {
        return Some(config.pandoc_bin.clone());
    }
    which::which("pandoc").ok().map(|p| p.display().to_string())
}

/// Register the `doc.*` family. Returns immediately without adding any tool if
/// pandoc cannot be found, logging a warning for the operator.
pub fn register(reg: &mut ToolRegistry, config: &DocConfig) {
    let Some(bin) = resolve_pandoc(config) else {
        tracing::warn!(
            "doc.enabled=true but pandoc not found in PATH \
             (set doc.pandoc_bin in config to point at it explicitly): \
             doc.to_docx will not be registered"
        );
        return;
    };
    let pandoc = Arc::new(PandocCtx { bin, timeout_secs: config.pandoc_timeout_secs });

    reg.add(
        ToolSchema::new("doc.to_docx", DESCRIPTION)
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str(
                "src_path",
                "Absolute POSIX path of the Markdown or HTML source file.",
            )
            .req_str("dst_path", "Absolute POSIX path of the .docx file to write.")
            .opt_str_null(
                "template_path",
                "Absolute POSIX path of a .docx template in the volume (optional).",
            )
            .opt_bool(
                "overwrite",
                false,
                "Allow overwriting an existing file (default no-clobber).",
            ),
        handler(move |ctx, a| {
            let pandoc = pandoc.clone();
            async move {
                let (mount, client) = volume(&ctx, &a).await?;
                let src_path = norm(&ctx, &a, "src_path")?;
                let dst_path = norm(&ctx, &a, "dst_path")?;
                let template_path = match a.opt_str("template_path") {
                    Some(p) => Some(ctx.state.safety.normalize_path(&p)?),
                    None => None,
                };
                let overwrite = a.bool_or("overwrite", false);
                to_docx(
                    &WriteCtx {
                        person: &ctx.person,
                        mount: &mount,
                        safety: &ctx.state.safety,
                    },
                    &client,
                    &pandoc,
                    &src_path,
                    &dst_path,
                    template_path.as_deref(),
                    overwrite,
                )
                .await
            }
        }),
    );
}

/// Caller identity and safety context bundled to stay within clippy's argument limit.
struct WriteCtx<'a> {
    person: &'a str,
    mount: &'a str,
    safety: &'a Arc<SafetyManager>,
}

async fn to_docx(
    ctx: &WriteCtx<'_>,
    client: &Arc<VolumeClient>,
    pandoc: &PandocCtx,
    src_path: &str,
    dst_path: &str,
    template_path: Option<&str>,
    overwrite: bool,
) -> Result<serde_json::Value> {
    let (person, mount, safety) = (ctx.person, ctx.mount, ctx.safety);
    // 1. Extension checks.
    validate_dst_ext(dst_path)?;
    validate_src_ext(src_path)?;

    // 2. Read source from the volume.
    let src_bytes = client.read_bytes(src_path).await.map_err(|_| {
        ToolError::not_found(format!("source file not found: {src_path}"))
    })?;

    // 3. Optionally read the template from the volume.
    let template_bytes = match template_path {
        Some(tp) => {
            let b = client.read_bytes(tp).await.map_err(|_| {
                ToolError::not_found(format!("template file not found: {tp}"))
            })?;
            Some(b)
        }
        None => None,
    };

    // 4. No-clobber check.
    let already_exists = client.exists(dst_path).await.unwrap_or(false);
    if already_exists && !overwrite {
        return Err(ToolError::no_clobber(format!(
            "file already exists (set overwrite=true to replace): {dst_path}"
        )));
    }
    // Read-before-write guard: only enforced when overwriting an existing file.
    if already_exists {
        safety.record_read(person, mount, dst_path);
        safety.ensure_read_before_write(person, mount, dst_path)?;
    }

    // 5. Build a temp directory and write input files.
    let src_ext = std::path::Path::new(src_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("md");
    let tmpdir = tempfile::tempdir().map_err(|e| {
        ToolError::internal(format!("cannot create temp directory: {e}"))
    })?;
    let input_path = tmpdir.path().join(format!("input.{src_ext}"));
    let output_path = tmpdir.path().join("output.docx");

    std::fs::write(&input_path, &src_bytes).map_err(|e| {
        ToolError::internal(format!("cannot write temp input: {e}"))
    })?;

    let template_arg: Option<std::path::PathBuf> = match template_bytes.as_ref() {
        Some(b) => {
            let tp = tmpdir.path().join("template.docx");
            std::fs::write(&tp, b).map_err(|e| {
                ToolError::internal(format!("cannot write temp template: {e}"))
            })?;
            Some(tp)
        }
        None => None,
    };

    // 6. Run pandoc.
    let docx_bytes = run_pandoc(pandoc, &input_path, &output_path, template_arg.as_deref()).await?;

    // 7. Write into the volume.
    client
        .write_bytes_atomic(dst_path, &docx_bytes)
        .await
        .map_err(|e| ToolError::internal(format!("failed to write output to volume: {e}")))?;

    // 8. Accounting.
    safety.record_read(person, mount, src_path);
    let n = docx_bytes.len();
    safety.charge_write(person, mount, n as i64)?;
    safety.record_audit(person, mount, "doc.to_docx", dst_path, &format!("{n} bytes"));

    Ok(json!({
        "path": dst_path,
        "bytes_written": n,
        "overwritten": already_exists,
    }))
}

/// Invoke pandoc and return the produced `.docx` bytes.
///
/// tmpdir is kept alive by the caller (via `TempDir` drop); this function only needs
/// the concrete file paths.
async fn run_pandoc(
    pandoc: &PandocCtx,
    input: &std::path::Path,
    output: &std::path::Path,
    template: Option<&std::path::Path>,
) -> Result<Vec<u8>> {
    let mut cmd = tokio::process::Command::new(&pandoc.bin);
    cmd.arg(input).arg("-o").arg(output);
    if let Some(tp) = template {
        cmd.arg(format!("--reference-doc={}", tp.display()));
    }
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let child = cmd.spawn().map_err(|e| {
        ToolError::internal(format!("cannot start pandoc: {e}"))
    })?;

    let out = tokio::time::timeout(
        Duration::from_secs(pandoc.timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| ToolError::internal(format!("pandoc timed out after {}s", pandoc.timeout_secs)))?
    .map_err(|e| ToolError::internal(format!("pandoc I/O error: {e}")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ToolError::internal(format!(
            "pandoc exited with {}: {stderr}",
            out.status
        )));
    }

    let bytes = std::fs::read(output).map_err(|e| {
        ToolError::internal(format!("cannot read pandoc output: {e}"))
    })?;

    if bytes.len() < 2 || &bytes[..2] != b"PK" {
        return Err(ToolError::internal(
            "pandoc output is not a valid .docx (missing ZIP magic PK)".to_string(),
        ));
    }
    Ok(bytes)
}

fn validate_dst_ext(path: &str) -> Result<()> {
    if std::path::Path::new(path).extension().and_then(|e| e.to_str()) != Some("docx") {
        return Err(ToolError::invalid_argument("path must end with .docx".to_string()));
    }
    Ok(())
}

fn validate_src_ext(path: &str) -> Result<()> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "md" | "markdown" | "html" | "htm" => Ok(()),
        _ => Err(ToolError::invalid_argument(
            "src_path must be a .md, .markdown, .html or .htm file".to_string(),
        )),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::testkit::{MOUNT, assert_description, assert_family, assert_schema, harness_with_extra};
    use serde_json::json;

    /// Wrapper compatible with `assert_family` / `assert_schema` / `assert_description`
    /// (signature: `fn(&mut ToolRegistry)`).
    fn register_with_pandoc(reg: &mut ToolRegistry) {
        let bin = which::which("pandoc").ok().map(|p| p.display().to_string());
        let Some(bin) = bin else {
            eprintln!("skipped: pandoc not found in PATH");
            return;
        };
        let cfg = DocConfig { enabled: true, pandoc_bin: bin, ..DocConfig::default() };
        register(reg, &cfg);
    }

    /// Wrapper with the `harness_with_extra` signature: `fn(&mut ToolRegistry, &ServerConfig)`.
    fn register_with_pandoc_extra(reg: &mut ToolRegistry, _cfg: &crate::config::ServerConfig) {
        register_with_pandoc(reg);
    }

    fn pandoc_available() -> bool {
        which::which("pandoc").is_ok()
    }

    // ── schema / contract tests (no pandoc needed for the asserts, but register skips) ────

    #[test]
    fn family_registers_every_tool() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        assert_family(register_with_pandoc, &["doc.to_docx"]);
    }

    #[test]
    fn doc_to_docx_schema_matches_contract() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        assert_schema(
            register_with_pandoc,
            "doc.to_docx",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "src_path":{"description":"Absolute POSIX path of the Markdown or HTML source file.","type":"string"},
                 "dst_path":{"description":"Absolute POSIX path of the .docx file to write.","type":"string"},
                 "template_path":{"description":"Absolute POSIX path of a .docx template in the volume (optional).","type":"string","default":null},
                 "overwrite":{"description":"Allow overwriting an existing file (default no-clobber).","type":"boolean","default":false}},
               "required":["mount_id","src_path","dst_path"]}"#,
        );
    }

    #[test]
    fn doc_to_docx_description_matches_contract() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        assert_description(register_with_pandoc, "doc.to_docx", DESCRIPTION);
    }

    // ── integration tests (pandoc required) ────────────────────────────────────

    #[tokio::test]
    async fn doc_to_docx_converts_markdown() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# Title\n\nBody text.\n").await;
        let r = h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx"
        })).await.unwrap();
        assert_eq!(r["path"], "/out.docx");
        assert_eq!(r["overwritten"], false);
        assert!(r["bytes_written"].as_i64().unwrap() > 0);
        let bytes = h.client().await.read_bytes("/out.docx").await.unwrap();
        assert_eq!(&bytes[..2], b"PK", "output must be a ZIP/docx archive");
    }

    #[tokio::test]
    async fn doc_to_docx_converts_html() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/page.html", "<h1>Title</h1><p>Body.</p>").await;
        let r = h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/page.html", "dst_path": "/out.docx"
        })).await.unwrap();
        assert_eq!(r["path"], "/out.docx");
        let bytes = h.client().await.read_bytes("/out.docx").await.unwrap();
        assert_eq!(&bytes[..2], b"PK");
    }

    #[tokio::test]
    async fn doc_to_docx_no_clobber() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# A\n").await;
        h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx"
        })).await.unwrap();
        let err = h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx"
        })).await.unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn doc_to_docx_overwrite_allowed() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# A\n").await;
        h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx"
        })).await.unwrap();
        let r = h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx", "overwrite": true
        })).await.unwrap();
        assert_eq!(r["overwritten"], true);
    }

    #[tokio::test]
    async fn doc_to_docx_rejects_wrong_dst_ext() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# A\n").await;
        let err = h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.txt"
        })).await.unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert_eq!(err.message, "path must end with .docx");
    }

    #[tokio::test]
    async fn doc_to_docx_rejects_wrong_src_ext() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/data.csv", "a,b\n1,2\n").await;
        let err = h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/data.csv", "dst_path": "/out.docx"
        })).await.unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(err.message.contains("src_path must be a .md"));
    }

    #[tokio::test]
    async fn doc_to_docx_missing_src() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        let err = h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/does_not_exist.md", "dst_path": "/out.docx"
        })).await.unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }

    #[tokio::test]
    async fn doc_to_docx_missing_template() {
        if !pandoc_available() { eprintln!("skipped: pandoc not found"); return; }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# A\n").await;
        let err = h.call("doc.to_docx", json!({
            "mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx",
            "template_path": "/no_such_template.docx"
        })).await.unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }
}
