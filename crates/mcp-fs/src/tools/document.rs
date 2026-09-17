//! Document family: `fs.extract_text`, `fs.write_docx`, `fs.documentize`.
//!
//! Port of the C# `Tools/DocumentTools.cs` plus the safety accounting the C#
//! `FsOps.ExtractDocument` / `FsOps.WriteDocx` wrap around the engines in
//! [`crate::docs`]. `fs.documentize` has no C# counterpart: it is the retry
//! surface of the external document service, and it writes the companion at the
//! very path `fs.extract_text` reads, so the two never produce two files.

use crate::core::fs_ops;
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::tools::{norm, volume};

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
            fs_ops::extract_document(
                &client,
                &ctx.state.safety,
                &ctx.state.config.extract.ocr,
                &ctx.person,
                &mount,
                &path,
                a.int_or("max_chars", 200_000).max(0) as usize,
                a.int_or("preview_chars", 4_000).max(0) as usize,
                a.bool_or("ocr", true),
                a.bool_or("refresh", false),
            )
            .await
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
        .opt_bool(
            "overwrite",
            false,
            "Allow overwriting an existing file (default no-clobber).",
        ),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            let out = fs_ops::write_docx(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &a.str("markdown")?,
                a.opt_str("title").as_deref(),
                a.bool_or("overwrite", false),
            )
            .await?;
            // A .docx is a zip, so the reread skips it. The hook is here for the
            // same reason as on every other write tool: one rule, no exceptions
            // to remember when the extraction story changes.
            crate::search::indexer::after_write_reread(&ctx.state, &mount, &path, &client).await;
            Ok(out)
        }),
    );

    reg.add(
        ToolSchema::new("fs.documentize", "Generate the Markdown companion of a stored document.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str(
                "path",
                "Absolute POSIX path of the stored document to convert. PowerPoint, Word, \
             PDF, audio and video only.",
            )
            .opt_bool(
                "overwrite",
                false,
                "Allow overwriting an existing companion .md (default no-clobber).",
            ),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::documentize(
                &client,
                &ctx.state.safety,
                ctx.state.doc_service.as_deref(),
                &ctx.person,
                &mount,
                &path,
                a.bool_or("overwrite", false),
            )
            .await
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::service::StubDocService;
    use crate::errors::code;
    use crate::tools::testkit::{
        MOUNT, PERSON, assert_description, assert_family, assert_schema, harness,
        harness_with_doc_service,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;

    const NAMES: &[&str] = &["fs.extract_text", "fs.write_docx", "fs.documentize"];

    /// What the injected stub converts every document to.
    const STUB_MD: &str = "# converted by the stub\n";

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
        let r = h
            .call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/data.csv"}))
            .await
            .unwrap();
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

        let again = h
            .call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/data.csv"}))
            .await
            .unwrap();
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
        let r = h
            .call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/notes.txt"}))
            .await
            .unwrap();
        assert_eq!(r["md_path"], Value::Null);
        assert_eq!(r["format"], "text");
        assert_eq!(h.state.safety.bytes_written(PERSON, MOUNT), 0);
    }

    #[tokio::test]
    async fn extract_text_rejects_audio_and_a_directory() {
        let h = harness().await;
        h.seed("/talk.mp3", "not really audio\n").await;
        let err = h
            .call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/talk.mp3"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);

        h.client().await.makedirs("/dir", true).await.unwrap();
        let err = h
            .call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/dir"}))
            .await
            .unwrap_err();
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

    // ── fs.documentize ────────────────────────────────────────────────────────

    #[test]
    fn fs_documentize_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.documentize",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path of the stored document to convert. PowerPoint, Word, PDF, audio and video only.","type":"string"},
                 "overwrite":{"description":"Allow overwriting an existing companion .md (default no-clobber).","type":"boolean","default":false}},
               "required":["mount_id","path"]}"#,
        );
        assert_description(
            register,
            "fs.documentize",
            "Generate the Markdown companion of a stored document.",
        );
    }

    /// The retry surface: it converts what is already stored and, unlike an
    /// upload, honours the caller's `overwrite` on the companion.
    #[tokio::test]
    async fn documentize_writes_the_companion_and_is_no_clobber() {
        let h = harness_with_doc_service(|_| {}, Some(Arc::new(StubDocService::ok(STUB_MD)))).await;
        h.client().await.write_bytes_atomic("/report.pdf", b"%PDF").await.unwrap();

        let r = h
            .call("fs.documentize", json!({"mount_id": MOUNT, "path": "/report.pdf"}))
            .await
            .unwrap();
        assert_eq!(r["path"], "/report.pdf");
        assert_eq!(r["md_path"], "/report.md");
        assert_eq!(r["bytes_written"], STUB_MD.len());
        assert_eq!(r["overwritten"], false);
        assert_eq!(h.client().await.read_text("/report.md").await.unwrap(), STUB_MD);

        let err = h
            .call("fs.documentize", json!({"mount_id": MOUNT, "path": "/report.pdf"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);

        let again = h
            .call(
                "fs.documentize",
                json!({"mount_id": MOUNT, "path": "/report.pdf", "overwrite": true}),
            )
            .await
            .unwrap();
        assert_eq!(again["overwritten"], true);
    }

    /// The companion lands where `fs.extract_text` looks, so the built-in
    /// extractor serves it as a cache hit instead of producing a second file.
    #[tokio::test]
    async fn a_documentized_companion_is_a_cache_hit_for_extract_text() {
        let h = harness_with_doc_service(|_| {}, Some(Arc::new(StubDocService::ok(STUB_MD)))).await;
        h.client().await.write_bytes_atomic("/report.pdf", b"%PDF-1.4 not really").await.unwrap();
        h.call("fs.documentize", json!({"mount_id": MOUNT, "path": "/report.pdf"})).await.unwrap();

        let r = h
            .call("fs.extract_text", json!({"mount_id": MOUNT, "path": "/report.pdf"}))
            .await
            .unwrap();
        assert_eq!(r["cached"], true);
        assert_eq!(r["preview"], STUB_MD);
    }

    #[tokio::test]
    async fn documentize_refuses_an_ineligible_extension_and_a_missing_service() {
        let h = harness_with_doc_service(|_| {}, Some(Arc::new(StubDocService::ok(STUB_MD)))).await;
        h.seed("/notes.txt", "plain").await;
        let err = h
            .call("fs.documentize", json!({"mount_id": MOUNT, "path": "/notes.txt"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);
        assert!(!h.client().await.exists("/notes.md").await.unwrap());

        let off = harness().await;
        off.client().await.write_bytes_atomic("/report.pdf", b"%PDF").await.unwrap();
        let err = off
            .call("fs.documentize", json!({"mount_id": MOUNT, "path": "/report.pdf"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);
        assert!(err.message.contains("not configured"), "{}", err.message);
    }

    #[tokio::test]
    async fn documentize_on_a_missing_file_is_not_found() {
        let h = harness_with_doc_service(|_| {}, Some(Arc::new(StubDocService::ok(STUB_MD)))).await;
        let err = h
            .call("fs.documentize", json!({"mount_id": MOUNT, "path": "/nope.pdf"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }
}
