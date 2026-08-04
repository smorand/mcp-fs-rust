//! Document family: `fs.extract_text`, `fs.write_docx`.
//!
//! Port of the C# `Tools/DocumentTools.cs` plus the safety accounting the C#
//! `FsOps.ExtractDocument` / `FsOps.WriteDocx` wrap around the engines in
//! [`crate::docs`].

use crate::errors::{Result, ToolError};
use crate::mcp::registry::{ToolCtx, ToolRegistry, handler};
use crate::mcp::ToolSchema;
use crate::storage::VolumeClient;
use crate::tools::{norm, volume};
use serde_json::{Value, json};

/// The C# description is one concatenated string; it is the LLM facing doc for
/// the whole extraction pipeline, so it is reproduced verbatim.
const EXTRACT_DESC: &str = "Extract a document to Markdown and store it as a companion .md next to the source \
(report.pdf -> report.md), reusing it if already up to date. Returns md_path + a preview; \
read the .md with fs.read for the full content. Handles PDF, DOCX, PPTX, XLSX, HTML, CSV, \
images (OCR via a configured multimodal provider) and text; audio/video unsupported.";

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new("fs.extract_text", EXTRACT_DESC)
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path of the source document.")
            .opt_int("max_chars", 200_000, "Maximum characters of Markdown to store.")
            .opt_int("preview_chars", 4_000, "Number of leading characters returned as a preview.")
            .opt_bool("ocr", true, "Enable OCR for images via a configured multimodal provider.")
            .opt_bool(
                "refresh",
                false,
                "Force re-extraction even if the companion .md is up to date.",
            ),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            let ocr = crate::docs::provider_from_config(&ctx.state.config.extract.ocr);
            let payload = crate::docs::extract_text(
                &client,
                ocr.as_ref(),
                &path,
                a.int_or("max_chars", 200_000).max(0) as usize,
                a.int_or("preview_chars", 4_000).max(0) as usize,
                a.bool_or("ocr", true),
                a.bool_or("refresh", false),
            )
            .await?;
            account_for_companion(&ctx, &client, &mount, &payload).await?;
            Ok(payload)
        }),
    );

    reg.add(
        ToolSchema::new(
            "fs.write_docx",
            "Render Markdown into a .docx Word document and write it to the volume.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Absolute POSIX path of the .docx file to write.")
        .req_str("markdown", "Markdown source rendered into the Word document.")
        .opt_str_null("title", "Optional document title.")
        .opt_bool("overwrite", false, "Allow overwriting an existing file (default no-clobber)."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            write_docx(
                &ctx,
                &client,
                &mount,
                &path,
                &a.str("markdown")?,
                a.opt_str("title").as_deref(),
                a.bool_or("overwrite", false),
            )
            .await
        }),
    );
}

/// Charge the companion `.md`, mark it read and audit it.
///
/// Deviation from the C#, deliberate: the Rust engine owns the companion write,
/// so the quota is charged just after that write instead of just before it. The
/// observable payload is identical; the only difference is that a session which
/// blows its quota on the last extraction still leaves the `.md` on disk.
async fn account_for_companion(
    ctx: &ToolCtx,
    client: &VolumeClient,
    mount: &str,
    payload: &Value,
) -> Result<()> {
    // A cache hit wrote nothing, and a null md_path means the extraction produced
    // no text worth storing.
    if payload["cached"] == Value::Bool(true) {
        return Ok(());
    }
    let Some(md) = payload["md_path"].as_str() else {
        return Ok(());
    };
    let size = client.stat(md).await?.size;
    let safety = &ctx.state.safety;
    safety.charge_write(&ctx.person, mount, size)?;
    // Recording the read lets a follow-up fs.edit on the companion pass the guard.
    safety.record_read(&ctx.person, mount, md);
    safety.record_audit(&ctx.person, mount, "extract_text", md, &format!("{size} bytes"));
    Ok(())
}

/// Render Markdown to a `.docx` and store it. Keys: `path`, `bytes_written`,
/// `overwritten`.
async fn write_docx(
    ctx: &ToolCtx,
    client: &VolumeClient,
    mount: &str,
    norm: &str,
    markdown: &str,
    title: Option<&str>,
    overwrite: bool,
) -> Result<Value> {
    if !norm.to_ascii_lowercase().ends_with(".docx") {
        return Err(ToolError::invalid_argument("path must end with .docx"));
    }
    let exists = client.exists(norm).await?;
    if exists && !overwrite {
        return Err(ToolError::no_clobber(format!("'{norm}' exists (pass overwrite=true)")));
    }
    let safety = &ctx.state.safety;
    if exists {
        safety.ensure_read_before_write(&ctx.person, mount, norm)?;
    }
    let data = crate::docs::render_markdown_to_docx(markdown, title)?;
    let parent = match norm.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => norm[..i].to_string(),
    };
    if parent != "/" {
        client.makedirs(&parent, true).await?;
    }
    safety.charge_write(&ctx.person, mount, data.len() as i64)?;
    client.write_bytes_atomic(norm, &data).await?;
    safety.record_read(&ctx.person, mount, norm);
    safety.record_audit(&ctx.person, mount, "write_docx", norm, &format!("{} bytes", data.len()));
    Ok(json!({"path": norm, "bytes_written": data.len(), "overwritten": exists}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::testkit::{
        MOUNT, PERSON, assert_description, assert_family, assert_schema, harness,
    };

    const NAMES: &[&str] = &["fs.extract_text", "fs.write_docx"];

    #[test]
    fn family_registers_every_tool() {
        assert_family(register, NAMES);
    }

    #[test]
    fn fs_extract_text_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.extract_text",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path of the source document.","type":"string"},
                 "max_chars":{"description":"Maximum characters of Markdown to store.","type":"integer","default":200000},
                 "preview_chars":{"description":"Number of leading characters returned as a preview.","type":"integer","default":4000},
                 "ocr":{"description":"Enable OCR for images via a configured multimodal provider.","type":"boolean","default":true},
                 "refresh":{"description":"Force re-extraction even if the companion .md is up to date.","type":"boolean","default":false}},
               "required":["mount_id","path"]}"#,
        );
    }

    /// The long description is compared in full: it is the tool's LLM facing doc.
    #[test]
    fn fs_extract_text_description_matches_the_contract() {
        assert_description(
            register,
            "fs.extract_text",
            "Extract a document to Markdown and store it as a companion .md next to the source \
             (report.pdf -> report.md), reusing it if already up to date. Returns md_path + a preview; \
             read the .md with fs.read for the full content. Handles PDF, DOCX, PPTX, XLSX, HTML, CSV, \
             images (OCR via a configured multimodal provider) and text; audio/video unsupported.",
        );
    }

    #[test]
    fn fs_write_docx_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.write_docx",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path of the .docx file to write.","type":"string"},
                 "markdown":{"description":"Markdown source rendered into the Word document.","type":"string"},
                 "title":{"description":"Optional document title.","type":"string","default":null},
                 "overwrite":{"description":"Allow overwriting an existing file (default no-clobber).","type":"boolean","default":false}},
               "required":["mount_id","path","markdown"]}"#,
        );
        assert_description(
            register,
            "fs.write_docx",
            "Render Markdown into a .docx Word document and write it to the volume.",
        );
    }

    #[tokio::test]
    async fn extract_text_writes_a_companion_and_audits_it() {
        let h = harness().await;
        h.seed("/data.csv", "a,b\n1,2\n").await;
        let r = h.call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/data.csv"})).await.unwrap();
        assert_eq!(r["md_path"], "/data.md");
        assert_eq!(r["format"], "csv");
        assert_eq!(r["cached"], false);
        assert!(h.client().await.exists("/data.md").await.unwrap());

        let audit = h.state.safety.audit(PERSON, MOUNT);
        let last = audit.last().unwrap();
        assert_eq!(last.op, "extract_text");
        assert_eq!(last.path, "/data.md");
        assert!(last.detail.ends_with(" bytes"));
        assert!(h.state.safety.bytes_written(PERSON, MOUNT) > 0);
    }

    #[tokio::test]
    async fn extract_text_reuses_an_up_to_date_companion() {
        let h = harness().await;
        h.seed("/data.csv", "a,b\n1,2\n").await;
        h.call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/data.csv"})).await.unwrap();
        let charged = h.state.safety.bytes_written(PERSON, MOUNT);

        let again = h.call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/data.csv"})).await.unwrap();
        assert_eq!(again["cached"], true);
        assert_eq!(again["format"], "md");
        // A cache hit writes nothing, so nothing is charged.
        assert_eq!(h.state.safety.bytes_written(PERSON, MOUNT), charged);
    }

    /// A plain text file has no companion extension, so `md_path` stays null and
    /// no quota is charged.
    #[tokio::test]
    async fn extract_text_on_a_text_file_has_no_companion() {
        let h = harness().await;
        h.seed("/notes.txt", "hello\n").await;
        let r = h.call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/notes.txt"})).await.unwrap();
        assert_eq!(r["md_path"], Value::Null);
        assert_eq!(r["format"], "text");
        assert_eq!(h.state.safety.bytes_written(PERSON, MOUNT), 0);
    }

    #[tokio::test]
    async fn extract_text_rejects_audio_and_a_directory() {
        let h = harness().await;
        h.seed("/talk.mp3", "not really audio\n").await;
        let err = h.call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/talk.mp3"})).await.unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);

        h.client().await.makedirs("/dir", true).await.unwrap();
        let err = h.call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/dir"})).await.unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }

    #[tokio::test]
    async fn write_docx_produces_a_zip_and_is_no_clobber() {
        let h = harness().await;
        let r = h
            .call(
                "fs.write_docx",
                json!({"mount_id": MOUNT, "path": "/reports/out.docx",
                       "markdown": "# Title\n\nBody text.\n", "title": "Report"}),
            )
            .await
            .unwrap();
        assert_eq!(r["path"], "/reports/out.docx");
        assert_eq!(r["overwritten"], false);
        assert!(r["bytes_written"].as_i64().unwrap() > 0);

        let bytes = h.client().await.read_bytes("/reports/out.docx").await.unwrap();
        assert_eq!(&bytes[..2], b"PK", "a .docx is a zip archive");

        let err = h
            .call(
                "fs.write_docx",
                json!({"mount_id": MOUNT, "path": "/reports/out.docx", "markdown": "# Again\n"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);
    }

    /// The write records a read, so the immediate overwrite passes the guard.
    #[tokio::test]
    async fn write_docx_can_overwrite_its_own_output() {
        let h = harness().await;
        h.call("fs.write_docx", json!({"mount_id": MOUNT, "path": "/o.docx", "markdown": "a"}))
            .await
            .unwrap();
        let r = h
            .call(
                "fs.write_docx",
                json!({"mount_id": MOUNT, "path": "/o.docx", "markdown": "b", "overwrite": true}),
            )
            .await
            .unwrap();
        assert_eq!(r["overwritten"], true);
    }

    #[tokio::test]
    async fn write_docx_requires_the_docx_extension() {
        let h = harness().await;
        let err = h
            .call("fs.write_docx", json!({"mount_id": MOUNT, "path": "/out.txt", "markdown": "x"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert_eq!(err.message, "path must end with .docx");
    }
}
