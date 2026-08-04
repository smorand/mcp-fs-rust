//! Write family: `fs.write` (no-clobber and atomic), `fs.append`,
//! `fs.create_empty`.
//!
//! Port of the C# `Tools/WriteTools.cs`.

use crate::core::fs_ops;
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::tools::{norm, volume};

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
            fs_ops::write_text(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &a.str("content")?,
                a.bool_or("overwrite", false),
                a.bool_or("create_parents", true),
            )
            .await
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
            fs_ops::append_text(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                &a.str("content")?,
                a.bool_or("create", false),
            )
            .await
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::testkit::{MOUNT, assert_description, assert_family, assert_schema, harness};
    use serde_json::json;

    const NAMES: &[&str] = &["fs.write", "fs.append", "fs.create_empty"];

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
