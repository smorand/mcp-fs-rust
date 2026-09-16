//! Write family: `fs.write` (no-clobber and atomic), `fs.append`,
//! `fs.create_empty`, `fs.write_bytes`.
//!
//! Port of the C# `Tools/WriteTools.cs`, plus `fs.write_bytes`, which has no C#
//! counterpart: it is the only way to put binary content in a volume through the
//! MCP surface, and it is where the document service flag lives.

use crate::core::fs_ops;
use crate::errors::ToolError;
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::search::indexer;
use crate::tools::{norm, volume};
use base64::Engine as _;

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new("fs.write", "Create or overwrite a file (no-clobber by default, atomic).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume, e.g. /src/app.py.")
            .req_str("content", "Full text content to write to the file.")
            .opt_bool("overwrite", false, "Allow overwriting an existing file (default no-clobber).")
            .opt_bool("create_parents", true, "Create missing parent directories."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            let content = a.str("content")?;
            let out = fs_ops::write_text(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &content,
                a.bool_or("overwrite", false),
                a.bool_or("create_parents", true),
            )
            .await?;
            // The new text is already in hand, so no read back is needed.
            indexer::after_write(&ctx.state, &mount, &path, &content).await;
            Ok(out)
        }),
    );

    reg.add(
        ToolSchema::new("fs.append", "Append content to a file (optionally create it).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_str("content", "Text content to append at the end of the file.")
            .opt_bool("create", false, "Create the file if it does not exist."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            let out = fs_ops::append_text(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &a.str("content")?,
                a.bool_or("create", false),
            )
            .await?;
            // Only the appended fragment is in hand, so the whole file is re-read.
            indexer::after_write_reread(&ctx.state, &mount, &path, &client).await;
            Ok(out)
        }),
    );

    reg.add(
        ToolSchema::new("fs.create_empty", "Create an empty file (touch).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path of the file to create.")
            .opt_bool("exist_ok", false, "Succeed silently if the file already exists."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::create_empty(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.bool_or("exist_ok", false),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.write_bytes", "Write raw bytes (base64) to a file.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_str("base64", "File content, base64 encoded.")
            .opt_bool("overwrite", false, "Allow overwriting an existing file (default no-clobber).")
            .opt_bool("create_parents", true, "Create missing parent directories.")
            .opt_bool(
                "trigger_documentation_service",
                false,
                "Also generate the Markdown companion (path.md) through the configured \
                 document service. Supported for PowerPoint, Word, PDF, audio and video only.",
            ),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            // The engine and the alphabet `fs.read_bytes` encodes with, so a read
            // then write round trip is byte identical.
            let data = base64::engine::general_purpose::STANDARD
                .decode(a.str("base64")?.trim())
                .map_err(|e| {
                    ToolError::invalid_argument(format!(
                        "argument 'base64' is not valid base64: {e}"
                    ))
                })?;
            let out = fs_ops::write_bytes_documented(
                &client,
                &ctx.state.safety,
                ctx.state.doc_service.as_deref(),
                &ctx.person,
                &mount,
                &path,
                &data,
                a.bool_or("overwrite", false),
                a.bool_or("create_parents", true),
                a.bool_or("trigger_documentation_service", false),
            )
            .await?;
            // A binary payload has no text to index and is skipped by the reread.
            indexer::after_write_reread(&ctx.state, &mount, &path, &client).await;
            Ok(out)
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

    const NAMES: &[&str] = &["fs.write", "fs.append", "fs.create_empty", "fs.write_bytes"];

    /// What the injected stub converts every document to.
    const STUB_MD: &str = "# converted by the stub\n";

    fn b64(data: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(data)
    }

    #[test]
    fn family_registers_every_tool() {
        assert_family(register, NAMES);
    }

    #[test]
    fn fs_write_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.write",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume, e.g. /src/app.py.","type":"string"},
                 "content":{"description":"Full text content to write to the file.","type":"string"},
                 "overwrite":{"description":"Allow overwriting an existing file (default no-clobber).","type":"boolean","default":false},
                 "create_parents":{"description":"Create missing parent directories.","type":"boolean","default":true}},
               "required":["mount_id","path","content"]}"#,
        );
        assert_description(
            register,
            "fs.write",
            "Create or overwrite a file (no-clobber by default, atomic).",
        );
    }

    #[test]
    fn fs_append_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.append",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"},
                 "content":{"description":"Text content to append at the end of the file.","type":"string"},
                 "create":{"description":"Create the file if it does not exist.","type":"boolean","default":false}},
               "required":["mount_id","path","content"]}"#,
        );
    }

    #[test]
    fn fs_create_empty_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.create_empty",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path of the file to create.","type":"string"},
                 "exist_ok":{"description":"Succeed silently if the file already exists.","type":"boolean","default":false}},
               "required":["mount_id","path"]}"#,
        );
    }

    #[tokio::test]
    async fn write_reports_bytes_and_no_clobber() {
        let h = harness().await;
        let r = h
            .call("fs.write", json!({"mount_id": MOUNT, "path": "/a.txt", "content": "hello world\n"}))
            .await
            .unwrap();
        assert_eq!(r["path"], "/a.txt");
        assert_eq!(r["bytes_written"], 12);
        assert_eq!(r["overwritten"], false);
        assert_eq!(r["diff"], "");

        let err = h
            .call("fs.write", json!({"mount_id": MOUNT, "path": "/a.txt", "content": "again"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn append_needs_create_for_a_missing_file() {
        let h = harness().await;
        let err = h
            .call("fs.append", json!({"mount_id": MOUNT, "path": "/new.txt", "content": "x"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);

        let r = h
            .call(
                "fs.append",
                json!({"mount_id": MOUNT, "path": "/new.txt", "content": "abc", "create": true}),
            )
            .await
            .unwrap();
        assert_eq!(r["bytes_appended"], 3);
    }

    #[tokio::test]
    async fn create_empty_is_idempotent_only_with_exist_ok() {
        let h = harness().await;
        let r = h.call("fs.create_empty", json!({"mount_id": MOUNT, "path": "/e.txt"})).await.unwrap();
        assert_eq!(r["created"], true);

        let err = h
            .call("fs.create_empty", json!({"mount_id": MOUNT, "path": "/e.txt"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);

        let again = h
            .call("fs.create_empty", json!({"mount_id": MOUNT, "path": "/e.txt", "exist_ok": true}))
            .await
            .unwrap();
        assert_eq!(again["created"], false);
    }

    // ── new tests ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn write_with_overwrite_false_on_existing_file_is_409() {
        let h = harness().await;
        h.seed("/exists.txt", "old content").await;
        let err = h
            .call("fs.write", json!({"mount_id": MOUNT, "path": "/exists.txt", "content": "new content"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);
    }

    #[tokio::test]
    async fn write_creates_nested_parents_automatically() {
        let h = harness().await;
        let r = h
            .call("fs.write", json!({"mount_id": MOUNT, "path": "/a/b/c/deep.txt", "content": "hello"}))
            .await
            .unwrap();
        assert_eq!(r["path"], "/a/b/c/deep.txt");
        assert_eq!(r["bytes_written"], 5);
        let text = h.client().await.read_text("/a/b/c/deep.txt").await.unwrap();
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn write_with_empty_content_is_zero_bytes() {
        let h = harness().await;
        let r = h
            .call("fs.write", json!({"mount_id": MOUNT, "path": "/empty.txt", "content": ""}))
            .await
            .unwrap();
        assert_eq!(r["bytes_written"], 0);
        let stat = h
            .call("fs.stat", json!({"mount_id": MOUNT, "path": "/empty.txt"}))
            .await
            .unwrap();
        assert_eq!(stat["size"], 0);
    }

    #[tokio::test]
    async fn write_quota_exceeded_returns_err_quota_exceeded() {
        use crate::tools::testkit::harness_with;
        let h = harness_with(|cfg| {
            cfg.safety.write_quota_bytes = 10;
        }).await;
        let err = h
            .call("fs.write", json!({"mount_id": MOUNT, "path": "/big.txt", "content": "x".repeat(100)}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::WRITE_QUOTA_EXCEEDED);
    }

    #[tokio::test]
    async fn append_to_a_directory_is_invalid_argument() {
        let h = harness().await;
        h.call("fs.mkdir", json!({"mount_id": MOUNT, "path": "/mydir"})).await.unwrap();
        let err = h
            .call("fs.append", json!({"mount_id": MOUNT, "path": "/mydir", "content": "x"}))
            .await
            .unwrap_err();
        // The storage layer returns ERR_INVALID_ARGUMENT for writing to a directory.
        assert_eq!(err.code, code::INVALID_ARGUMENT);
    }

    // ── fs.write_bytes ──────────────────────────────────────────────────────────

    #[test]
    fn fs_write_bytes_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.write_bytes",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"},
                 "base64":{"description":"File content, base64 encoded.","type":"string"},
                 "overwrite":{"description":"Allow overwriting an existing file (default no-clobber).","type":"boolean","default":false},
                 "create_parents":{"description":"Create missing parent directories.","type":"boolean","default":true},
                 "trigger_documentation_service":{"description":"Also generate the Markdown companion (path.md) through the configured document service. Supported for PowerPoint, Word, PDF, audio and video only.","type":"boolean","default":false}},
               "required":["mount_id","path","base64"]}"#,
        );
        assert_description(register, "fs.write_bytes", "Write raw bytes (base64) to a file.");
    }

    /// The flag defaults to off, so the tool is a plain binary write and nothing
    /// reaches the document service.
    #[tokio::test]
    async fn write_bytes_stores_the_decoded_bytes_and_no_companion() {
        let h = harness_with_doc_service(|_| {}, Some(Arc::new(StubDocService::ok(STUB_MD)))).await;
        let payload: Vec<u8> = vec![0x50, 0x4b, 0x03, 0x04, 0x00, 0xff];

        let r = h
            .call(
                "fs.write_bytes",
                json!({"mount_id": MOUNT, "path": "/deck.pptx", "base64": b64(&payload)}),
            )
            .await
            .unwrap();

        assert_eq!(r["path"], "/deck.pptx");
        assert_eq!(r["bytes_written"], 6);
        assert_eq!(r["overwritten"], false);
        assert_eq!(r["documentation"], Value::Null);
        assert_eq!(h.client().await.read_bytes("/deck.pptx").await.unwrap(), payload);
        assert!(!h.client().await.exists("/deck.md").await.unwrap());
    }

    /// The flag on: the source and its companion are both stored, and the
    /// companion sits at the path `fs.extract_text` reads.
    #[tokio::test]
    async fn write_bytes_with_the_flag_stores_the_markdown_companion() {
        let h = harness_with_doc_service(|_| {}, Some(Arc::new(StubDocService::ok(STUB_MD)))).await;

        let r = h
            .call(
                "fs.write_bytes",
                json!({"mount_id": MOUNT, "path": "/slides/deck.pptx", "base64": b64(b"PK-bytes"),
                       "trigger_documentation_service": true}),
            )
            .await
            .unwrap();

        assert_eq!(r["documentation"]["md_path"], "/slides/deck.md");
        assert_eq!(r["documentation"]["bytes_written"], STUB_MD.len());
        assert_eq!(h.client().await.read_text("/slides/deck.md").await.unwrap(), STUB_MD);

        let log = h.state.safety.audit(PERSON, MOUNT);
        assert_eq!(log.last().unwrap().op, "doc_service");
    }

    /// The eligibility gate runs before the write, so an ineligible extension
    /// leaves nothing behind rather than a file without its companion.
    #[tokio::test]
    async fn write_bytes_refuses_an_ineligible_extension_and_writes_nothing() {
        let h = harness_with_doc_service(|_| {}, Some(Arc::new(StubDocService::ok(STUB_MD)))).await;

        let err = h
            .call(
                "fs.write_bytes",
                json!({"mount_id": MOUNT, "path": "/notes.txt", "base64": b64(b"plain"),
                       "trigger_documentation_service": true}),
            )
            .await
            .unwrap_err();

        assert_eq!(err.code, code::NOT_SUPPORTED);
        assert!(!h.client().await.exists("/notes.txt").await.unwrap(), "nothing may be written");
    }

    /// The flag while the feature is off must say so, not store the file silently
    /// without its companion.
    #[tokio::test]
    async fn write_bytes_without_a_configured_service_is_not_supported() {
        let h = harness().await;
        let err = h
            .call(
                "fs.write_bytes",
                json!({"mount_id": MOUNT, "path": "/deck.pptx", "base64": b64(b"PK"),
                       "trigger_documentation_service": true}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);
        assert!(err.message.contains("not configured"), "{}", err.message);
        assert!(!h.client().await.exists("/deck.pptx").await.unwrap());
    }

    /// Plan test 9: a payload that is not base64 is the caller's mistake, so it is
    /// an invalid argument rather than a stored pile of garbage.
    #[tokio::test]
    async fn write_bytes_rejects_invalid_base64() {
        let h = harness().await;
        let err = h
            .call(
                "fs.write_bytes",
                json!({"mount_id": MOUNT, "path": "/bad.bin", "base64": "not base64 at all!"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(err.message.contains("base64"), "{}", err.message);
        assert!(!h.client().await.exists("/bad.bin").await.unwrap());
    }

    /// A fresh write counts as a read, so the overwrite passes the read guard.
    #[tokio::test]
    async fn overwrite_after_a_write_is_allowed_and_returns_a_diff() {
        let h = harness().await;
        h.call("fs.write", json!({"mount_id": MOUNT, "path": "/a.txt", "content": "one\n"}))
            .await
            .unwrap();
        let r = h
            .call(
                "fs.write",
                json!({"mount_id": MOUNT, "path": "/a.txt", "content": "two\n", "overwrite": true}),
            )
            .await
            .unwrap();
        assert_eq!(r["overwritten"], true);
        assert!(r["diff"].as_str().unwrap().contains("-one"));
    }
}
