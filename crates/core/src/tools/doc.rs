//! `doc.*` tools: document conversion via pandoc.
//!
//! The family is optional: `register` is a no-op when pandoc is not found in PATH (or at
//! the configured bin path). A warning is logged so the operator knows why the tool is
//! absent.

use crate::config::DocConfig;
use crate::errors::{Result, ToolError};
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::safety::SafetyManager;
use crate::storage::VolumeClient;
use crate::tools::{norm, volume};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const DOCX_DESC: &str = "Convert a Markdown or HTML file in the volume to a .docx Word \
    document using pandoc. Supply a .docx template via template_path to apply custom \
    styles, headers, and footers.";

const PPTX_DESC: &str = "Convert a Markdown or HTML file in the volume to a .pptx \
    PowerPoint presentation using pandoc. In Markdown, --- (horizontal rule) separates \
    slides and # headings set the slide title. Supply a .pptx template via template_path \
    to apply custom themes and layouts.";

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
             doc.to_docx and doc.to_pptx will not be registered"
        );
        return;
    };
    let pandoc = Arc::new(PandocCtx { bin, timeout_secs: config.pandoc_timeout_secs });

    // ── doc.to_docx ──────────────────────────────────────────────────────────
    reg.add(
        ToolSchema::new("doc.to_docx", DOCX_DESC)
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("src_path", "Absolute POSIX path of the Markdown or HTML source file.")
            .req_str("dst_path", "Absolute POSIX path of the .docx file to write.")
            .opt_str_null(
                "template_path",
                "Absolute POSIX path of a .docx template in the volume (optional).",
            )
            .opt_bool(
                "overwrite",
                false,
                "Allow overwriting an existing file (default no-clobber).",
            )
            .read_only(false)
            .destructive(true)
            .idempotent(true)
            .open_world(true),
        handler({
            let pandoc = pandoc.clone();
            move |ctx, a| {
                let pandoc = pandoc.clone();
                async move {
                    let (mount, client) = volume(&ctx, &a).await?;
                    let src_path = norm(&ctx, &a, "src_path")?;
                    let dst_path = norm(&ctx, &a, "dst_path")?;
                    let template_path = opt_norm(&ctx, &a, "template_path")?;
                    let overwrite = a.bool_or("overwrite", false);
                    convert(
                        &WriteCtx { person: &ctx.person, mount: &mount, safety: &ctx.state.safety },
                        &client,
                        &pandoc,
                        &ConvertPaths {
                            src_path: &src_path,
                            dst_path: &dst_path,
                            out_ext: "docx",
                            template_path: template_path.as_deref(),
                            overwrite,
                        },
                    )
                    .await
                }
            }
        }),
    );

    // ── doc.to_pptx ──────────────────────────────────────────────────────────
    reg.add(
        ToolSchema::new("doc.to_pptx", PPTX_DESC)
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("src_path", "Absolute POSIX path of the Markdown or HTML source file.")
            .req_str("dst_path", "Absolute POSIX path of the .pptx file to write.")
            .opt_str_null(
                "template_path",
                "Absolute POSIX path of a .pptx template in the volume (optional).",
            )
            .opt_bool(
                "overwrite",
                false,
                "Allow overwriting an existing file (default no-clobber).",
            )
            .read_only(false)
            .destructive(true)
            .idempotent(true)
            .open_world(true),
        handler({
            let pandoc = pandoc.clone();
            move |ctx, a| {
                let pandoc = pandoc.clone();
                async move {
                    let (mount, client) = volume(&ctx, &a).await?;
                    let src_path = norm(&ctx, &a, "src_path")?;
                    let dst_path = norm(&ctx, &a, "dst_path")?;
                    let template_path = opt_norm(&ctx, &a, "template_path")?;
                    let overwrite = a.bool_or("overwrite", false);
                    convert(
                        &WriteCtx { person: &ctx.person, mount: &mount, safety: &ctx.state.safety },
                        &client,
                        &pandoc,
                        &ConvertPaths {
                            src_path: &src_path,
                            dst_path: &dst_path,
                            out_ext: "pptx",
                            template_path: template_path.as_deref(),
                            overwrite,
                        },
                    )
                    .await
                }
            }
        }),
    );
}

/// Normalize an optional path parameter (returns `None` when the arg is absent/null).
fn opt_norm(
    ctx: &crate::mcp::registry::ToolCtx,
    a: &crate::mcp::Args,
    key: &str,
) -> Result<Option<String>> {
    match a.opt_str(key) {
        Some(p) => Ok(Some(ctx.state.safety.normalize_path(&p)?)),
        None => Ok(None),
    }
}

/// Caller identity and safety context bundled to stay within clippy's argument limit.
struct WriteCtx<'a> {
    person: &'a str,
    mount: &'a str,
    safety: &'a Arc<SafetyManager>,
}

/// Source/destination paths and output format, bundled to stay within clippy's argument limit.
struct ConvertPaths<'a> {
    src_path: &'a str,
    dst_path: &'a str,
    /// Expected output extension: `"docx"` or `"pptx"`.
    out_ext: &'a str,
    template_path: Option<&'a str>,
    overwrite: bool,
}

/// Core conversion shared by `doc.to_docx` and `doc.to_pptx`.
///
/// Pandoc infers the writer from the output file extension, so no `--to` flag is needed.
async fn convert(
    ctx: &WriteCtx<'_>,
    client: &Arc<VolumeClient>,
    pandoc: &PandocCtx,
    paths: &ConvertPaths<'_>,
) -> Result<serde_json::Value> {
    let (person, mount, safety) = (ctx.person, ctx.mount, ctx.safety);
    let ConvertPaths { src_path, dst_path, out_ext, template_path, overwrite } = paths;

    // 1. Extension checks.
    validate_dst_ext(dst_path, out_ext)?;
    validate_src_ext(src_path)?;

    // 2. Read source from the volume.
    let src_bytes = client
        .read_bytes(src_path)
        .await
        .map_err(|_| ToolError::not_found(format!("source file not found: {src_path}")))?;

    // 3. Optionally read the template from the volume.
    let template_bytes = match template_path {
        Some(tp) => {
            let b = client
                .read_bytes(tp)
                .await
                .map_err(|_| ToolError::not_found(format!("template file not found: {tp}")))?;
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
    if already_exists {
        safety.record_read(person, mount, dst_path);
        safety.ensure_read_before_write(person, mount, dst_path)?;
    }

    // 5. Build a temp directory and write input files.
    let src_ext =
        std::path::Path::new(src_path).extension().and_then(|e| e.to_str()).unwrap_or("md");
    let tmpdir = tempfile::tempdir()
        .map_err(|e| ToolError::internal(format!("cannot create temp directory: {e}")))?;
    let input_path = tmpdir.path().join(format!("input.{src_ext}"));
    let output_path = tmpdir.path().join(format!("output.{out_ext}"));

    std::fs::write(&input_path, &src_bytes)
        .map_err(|e| ToolError::internal(format!("cannot write temp input: {e}")))?;

    let template_arg: Option<std::path::PathBuf> = match template_bytes.as_ref() {
        Some(b) => {
            let tp = tmpdir.path().join(format!("template.{out_ext}"));
            std::fs::write(&tp, b)
                .map_err(|e| ToolError::internal(format!("cannot write temp template: {e}")))?;
            Some(tp)
        }
        None => None,
    };

    // 6. Run pandoc.
    let out_bytes = run_pandoc(pandoc, &input_path, &output_path, template_arg.as_deref()).await?;

    // 7. Write into the volume.
    client
        .write_bytes_atomic(dst_path, &out_bytes)
        .await
        .map_err(|e| ToolError::internal(format!("failed to write output to volume: {e}")))?;

    // 8. Accounting.
    safety.record_read(person, mount, src_path);
    let n = out_bytes.len();
    safety.charge_write(person, mount, n as i64)?;
    let op = format!("doc.to_{out_ext}");
    safety.record_audit(person, mount, &op, dst_path, &format!("{n} bytes"));

    Ok(json!({
        "path": dst_path,
        "bytes_written": n,
        "overwritten": already_exists,
    }))
}

/// Invoke pandoc and return the produced bytes.
///
/// The output format is inferred from `output`'s extension. `tmpdir` is kept alive
/// by the caller; this function only needs the concrete file paths.
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
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());

    let child =
        cmd.spawn().map_err(|e| ToolError::internal(format!("cannot start pandoc: {e}")))?;

    let out =
        tokio::time::timeout(Duration::from_secs(pandoc.timeout_secs), child.wait_with_output())
            .await
            .map_err(|_| {
                ToolError::internal(format!("pandoc timed out after {}s", pandoc.timeout_secs))
            })?
            .map_err(|e| ToolError::internal(format!("pandoc I/O error: {e}")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ToolError::internal(format!("pandoc exited with {}: {stderr}", out.status)));
    }

    let bytes = std::fs::read(output)
        .map_err(|e| ToolError::internal(format!("cannot read pandoc output: {e}")))?;

    if bytes.len() < 2 || &bytes[..2] != b"PK" {
        return Err(ToolError::internal(format!(
            "pandoc output is not a valid .{} (missing ZIP magic PK)",
            output.extension().and_then(|e| e.to_str()).unwrap_or("?")
        )));
    }
    Ok(bytes)
}

fn validate_dst_ext(path: &str, expected: &str) -> Result<()> {
    if std::path::Path::new(path).extension().and_then(|e| e.to_str()) != Some(expected) {
        return Err(ToolError::invalid_argument(format!("path must end with .{expected}")));
    }
    Ok(())
}

fn validate_src_ext(path: &str) -> Result<()> {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("");
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
    use crate::tools::testkit::{
        MOUNT, assert_description, assert_family, assert_schema, harness_with_extra,
    };
    use serde_json::json;

    fn register_with_pandoc(reg: &mut ToolRegistry) {
        let Some(bin) = which::which("pandoc").ok().map(|p| p.display().to_string()) else {
            eprintln!("skipped: pandoc not found in PATH");
            return;
        };
        register(reg, &DocConfig { enabled: true, pandoc_bin: bin, ..DocConfig::default() });
    }

    fn register_with_pandoc_extra(reg: &mut ToolRegistry, _cfg: &crate::config::ServerConfig) {
        register_with_pandoc(reg);
    }

    fn pandoc_available() -> bool {
        which::which("pandoc").is_ok()
    }

    // ── family / schema / description ─────────────────────────────────────────

    #[test]
    fn family_registers_every_tool() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        assert_family(register_with_pandoc, &["doc.to_docx", "doc.to_pptx"]);
    }

    #[test]
    fn doc_to_docx_schema_matches_contract() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
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
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        assert_description(register_with_pandoc, "doc.to_docx", DOCX_DESC);
    }

    #[test]
    fn doc_to_pptx_schema_matches_contract() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        assert_schema(
            register_with_pandoc,
            "doc.to_pptx",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "src_path":{"description":"Absolute POSIX path of the Markdown or HTML source file.","type":"string"},
                 "dst_path":{"description":"Absolute POSIX path of the .pptx file to write.","type":"string"},
                 "template_path":{"description":"Absolute POSIX path of a .pptx template in the volume (optional).","type":"string","default":null},
                 "overwrite":{"description":"Allow overwriting an existing file (default no-clobber).","type":"boolean","default":false}},
               "required":["mount_id","src_path","dst_path"]}"#,
        );
    }

    #[test]
    fn doc_to_pptx_description_matches_contract() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        assert_description(register_with_pandoc, "doc.to_pptx", PPTX_DESC);
    }

    // ── doc.to_docx integration ───────────────────────────────────────────────

    #[tokio::test]
    async fn doc_to_docx_converts_markdown() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# Title\n\nBody text.\n").await;
        let r = h
            .call(
                "doc.to_docx",
                json!({"mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx"}),
            )
            .await
            .unwrap();
        assert_eq!(r["path"], "/out.docx");
        assert_eq!(r["overwritten"], false);
        assert!(r["bytes_written"].as_i64().unwrap() > 0);
        assert_eq!(&h.client().await.read_bytes("/out.docx").await.unwrap()[..2], b"PK");
    }

    #[tokio::test]
    async fn doc_to_docx_converts_html() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/page.html", "<h1>Title</h1><p>Body.</p>").await;
        let r = h
            .call(
                "doc.to_docx",
                json!({"mount_id": MOUNT, "src_path": "/page.html", "dst_path": "/out.docx"}),
            )
            .await
            .unwrap();
        assert_eq!(r["path"], "/out.docx");
        assert_eq!(&h.client().await.read_bytes("/out.docx").await.unwrap()[..2], b"PK");
    }

    #[tokio::test]
    async fn doc_to_docx_no_clobber() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# A\n").await;
        h.call(
            "doc.to_docx",
            json!({"mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx"}),
        )
        .await
        .unwrap();
        let err = h
            .call(
                "doc.to_docx",
                json!({"mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn doc_to_docx_overwrite_allowed() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# A\n").await;
        h.call(
            "doc.to_docx",
            json!({"mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx"}),
        )
        .await
        .unwrap();
        let r = h.call("doc.to_docx", json!({"mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx", "overwrite": true})).await.unwrap();
        assert_eq!(r["overwritten"], true);
    }

    #[tokio::test]
    async fn doc_to_docx_rejects_wrong_dst_ext() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# A\n").await;
        let err = h
            .call(
                "doc.to_docx",
                json!({"mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.txt"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert_eq!(err.message, "path must end with .docx");
    }

    #[tokio::test]
    async fn doc_to_docx_rejects_wrong_src_ext() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/data.csv", "a,b\n1,2\n").await;
        let err = h
            .call(
                "doc.to_docx",
                json!({"mount_id": MOUNT, "src_path": "/data.csv", "dst_path": "/out.docx"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(err.message.contains("src_path must be a .md"));
    }

    #[tokio::test]
    async fn doc_to_docx_missing_src() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        let err = h
            .call(
                "doc.to_docx",
                json!({"mount_id": MOUNT, "src_path": "/nope.md", "dst_path": "/out.docx"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }

    #[tokio::test]
    async fn doc_to_docx_missing_template() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/doc.md", "# A\n").await;
        let err = h.call("doc.to_docx", json!({"mount_id": MOUNT, "src_path": "/doc.md", "dst_path": "/out.docx", "template_path": "/nope.docx"})).await.unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }

    // ── doc.to_pptx integration ───────────────────────────────────────────────

    #[tokio::test]
    async fn doc_to_pptx_converts_markdown() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/deck.md", "# Slide 1\n\nContent.\n\n---\n\n# Slide 2\n\nMore content.\n").await;
        let r = h
            .call(
                "doc.to_pptx",
                json!({"mount_id": MOUNT, "src_path": "/deck.md", "dst_path": "/out.pptx"}),
            )
            .await
            .unwrap();
        assert_eq!(r["path"], "/out.pptx");
        assert_eq!(r["overwritten"], false);
        assert!(r["bytes_written"].as_i64().unwrap() > 0);
        assert_eq!(&h.client().await.read_bytes("/out.pptx").await.unwrap()[..2], b"PK");
    }

    #[tokio::test]
    async fn doc_to_pptx_converts_html() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/deck.html", "<h1>Slide 1</h1><p>Content.</p><h1>Slide 2</h1><p>More.</p>").await;
        let r = h
            .call(
                "doc.to_pptx",
                json!({"mount_id": MOUNT, "src_path": "/deck.html", "dst_path": "/out.pptx"}),
            )
            .await
            .unwrap();
        assert_eq!(r["path"], "/out.pptx");
        assert_eq!(&h.client().await.read_bytes("/out.pptx").await.unwrap()[..2], b"PK");
    }

    #[tokio::test]
    async fn doc_to_pptx_no_clobber() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/deck.md", "# A\n").await;
        h.call(
            "doc.to_pptx",
            json!({"mount_id": MOUNT, "src_path": "/deck.md", "dst_path": "/out.pptx"}),
        )
        .await
        .unwrap();
        let err = h
            .call(
                "doc.to_pptx",
                json!({"mount_id": MOUNT, "src_path": "/deck.md", "dst_path": "/out.pptx"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn doc_to_pptx_overwrite_allowed() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/deck.md", "# A\n").await;
        h.call(
            "doc.to_pptx",
            json!({"mount_id": MOUNT, "src_path": "/deck.md", "dst_path": "/out.pptx"}),
        )
        .await
        .unwrap();
        let r = h.call("doc.to_pptx", json!({"mount_id": MOUNT, "src_path": "/deck.md", "dst_path": "/out.pptx", "overwrite": true})).await.unwrap();
        assert_eq!(r["overwritten"], true);
    }

    #[tokio::test]
    async fn doc_to_pptx_rejects_wrong_dst_ext() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/deck.md", "# A\n").await;
        let err = h
            .call(
                "doc.to_pptx",
                json!({"mount_id": MOUNT, "src_path": "/deck.md", "dst_path": "/out.pdf"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert_eq!(err.message, "path must end with .pptx");
    }

    #[tokio::test]
    async fn doc_to_pptx_rejects_wrong_src_ext() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/data.csv", "a,b\n1,2\n").await;
        let err = h
            .call(
                "doc.to_pptx",
                json!({"mount_id": MOUNT, "src_path": "/data.csv", "dst_path": "/out.pptx"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(err.message.contains("src_path must be a .md"));
    }

    #[tokio::test]
    async fn doc_to_pptx_missing_src() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        let err = h
            .call(
                "doc.to_pptx",
                json!({"mount_id": MOUNT, "src_path": "/nope.md", "dst_path": "/out.pptx"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }

    #[tokio::test]
    async fn doc_to_pptx_missing_template() {
        if !pandoc_available() {
            eprintln!("skipped: pandoc not found");
            return;
        }
        let h = harness_with_extra(register_with_pandoc_extra).await;
        h.seed("/deck.md", "# A\n").await;
        let err = h.call("doc.to_pptx", json!({"mount_id": MOUNT, "src_path": "/deck.md", "dst_path": "/out.pptx", "template_path": "/nope.pptx"})).await.unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }
}
