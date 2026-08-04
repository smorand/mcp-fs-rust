//! Read family: `fs.read`, `fs.read_bytes`, `fs.read_lines`, `fs.read_section`,
//! `fs.read_many`, `fs.head`, `fs.tail`, `fs.count_lines`.
//!
//! Port of the C# `Tools/ReadTools.cs`.

use crate::core::fs_ops;
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::tools::{norm, volume};

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new("fs.read", "Read a text file with line-numbered, paged output.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume, e.g. /src/app.py.")
            .opt_int("offset_lines", 0, "0-based line offset to start reading from.")
            .opt_int("limit_lines", 2000, "Maximum number of lines to return.")
            .opt_bool("line_numbered", true, "Prefix each line with its 1-based line number."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::read_window(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.int_or("offset_lines", 0),
                a.int_or("limit_lines", 2000),
                a.bool_or("line_numbered", true),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.read_bytes", "Read raw bytes (base64) with MIME type.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .opt_int("offset_bytes", 0, "0-based byte offset to start reading from.")
            .opt_int("length_bytes", 65536, "Maximum number of bytes to return."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::read_bytes_b64(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.int_or("offset_bytes", 0),
                a.int_or("length_bytes", 65536),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.read_lines", "Read an inclusive line range [start_line, end_line].")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_int("start_line", "First 1-based line to return (inclusive).")
            .req_int("end_line", "Last 1-based line to return (inclusive)."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::read_lines(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.int("start_line")?,
                a.int("end_line")?,
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.read_section", "Read the indentation block around an anchor line.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .req_int("anchor_line", "1-based line whose indentation block is returned.")
            .opt_int("max_lines", 200, "Maximum number of lines to return for the block."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::read_section(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.int("anchor_line")?,
                a.int_or("max_lines", 200),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.read_many", "Batch read several files with per-file error isolation.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str_array("paths", "Absolute POSIX paths to read, one entry per file.")
            .opt_int("per_file_cap_lines", 500, "Maximum number of lines returned per file."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            // The paths stay raw here on purpose: the engine normalizes each one
            // and reports a rejected path as a per-file error instead of failing
            // the whole batch.
            let paths = a.req_str_array("paths")?;
            fs_ops::read_many(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &paths,
                a.int_or("per_file_cap_lines", 500),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.head", "First N lines of a file.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .opt_int("lines", 20, "Number of leading lines to return."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::head(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.int_or("lines", 20),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.tail", "Last N lines of a file.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .opt_int("lines", 20, "Number of trailing lines to return."),
        handler(|ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::tail(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &path,
                a.int_or("lines", 20),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.count_lines", "Count lines without returning content.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume."),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::count_lines(&client, &path).await
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testkit::{
        MOUNT, assert_description, assert_family, assert_schema, harness,
    };
    use serde_json::json;

    const NAMES: &[&str] = &[
        "fs.read",
        "fs.read_bytes",
        "fs.read_lines",
        "fs.read_section",
        "fs.read_many",
        "fs.head",
        "fs.tail",
        "fs.count_lines",
    ];

    #[test]
    fn family_registers_every_tool() {
        assert_family(register, NAMES);
    }

    /// Captured from the live C# server (`parity-golden.json`, `tools_list`).
    #[test]
    fn fs_read_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.read",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume, e.g. /src/app.py.","type":"string"},
                 "offset_lines":{"description":"0-based line offset to start reading from.","type":"integer","default":0},
                 "limit_lines":{"description":"Maximum number of lines to return.","type":"integer","default":2000},
                 "line_numbered":{"description":"Prefix each line with its 1-based line number.","type":"boolean","default":true}},
               "required":["mount_id","path"]}"#,
        );
        assert_description(register, "fs.read", "Read a text file with line-numbered, paged output.");
    }

    #[test]
    fn fs_read_many_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.read_many",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "paths":{"description":"Absolute POSIX paths to read, one entry per file.","type":"array","items":{"type":"string"}},
                 "per_file_cap_lines":{"description":"Maximum number of lines returned per file.","type":"integer","default":500}},
               "required":["mount_id","paths"]}"#,
        );
    }

    #[test]
    fn fs_read_lines_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.read_lines",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"},
                 "start_line":{"description":"First 1-based line to return (inclusive).","type":"integer"},
                 "end_line":{"description":"Last 1-based line to return (inclusive).","type":"integer"}},
               "required":["mount_id","path","start_line","end_line"]}"#,
        );
    }

    #[tokio::test]
    async fn read_returns_numbered_content_through_the_registry() {
        let h = harness().await;
        h.seed("/a.txt", "hello\nworld\n").await;
        let r = h.call("fs.read", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        assert_eq!(r["content"], "1\thello\n2\tworld");
        assert_eq!(r["total_lines"], 2);
        assert_eq!(r["truncated"], false);
    }

    #[tokio::test]
    async fn read_bytes_returns_base64_and_mime() {
        let h = harness().await;
        h.seed("/a.txt", "hello\nworld\n").await;
        let r = h.call("fs.read_bytes", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        assert_eq!(r["base64"], "aGVsbG8Kd29ybGQK");
        assert_eq!(r["mime_type"], "text/plain");
        assert_eq!(r["length"], 12);
    }

    #[tokio::test]
    async fn read_many_isolates_a_bad_path() {
        let h = harness().await;
        h.seed("/a.txt", "one\n").await;
        let r = h
            .call("fs.read_many", json!({"mount_id": MOUNT, "paths": ["/a.txt", "/nope"]}))
            .await
            .unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!(files[0]["path"], "/a.txt");
        assert_eq!(files[1]["error"], "not found: /nope");
    }

    #[tokio::test]
    async fn head_tail_and_count_lines_agree() {
        let h = harness().await;
        h.seed("/a.txt", "one\ntwo\nthree\n").await;
        let head = h
            .call("fs.head", json!({"mount_id": MOUNT, "path": "/a.txt", "lines": 1}))
            .await
            .unwrap();
        assert_eq!(head["content"], "1\tone");
        let tail = h
            .call("fs.tail", json!({"mount_id": MOUNT, "path": "/a.txt", "lines": 1}))
            .await
            .unwrap();
        assert_eq!(tail["content"], "3\tthree");
        let count = h.call("fs.count_lines", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        assert_eq!(count["total_lines"], 3);
    }

    #[tokio::test]
    async fn read_section_and_read_lines_window_the_file() {
        let h = harness().await;
        h.seed("/src/app.py", "def hello(name):\n    total = 1\n    return total\n").await;
        let section = h
            .call("fs.read_section", json!({"mount_id": MOUNT, "path": "/src/app.py", "anchor_line": 2}))
            .await
            .unwrap();
        assert_eq!(section["start_line"], 1);
        assert_eq!(section["end_line"], 3);
        let lines = h
            .call(
                "fs.read_lines",
                json!({"mount_id": MOUNT, "path": "/src/app.py", "start_line": 2, "end_line": 2}),
            )
            .await
            .unwrap();
        assert_eq!(lines["content"], "2\t    total = 1");
        assert_eq!(lines["total_lines"], 3);
    }

    /// A relative path is normalized, and traversal cannot escape the root.
    #[tokio::test]
    async fn paths_are_normalized_before_the_engine_sees_them() {
        let h = harness().await;
        h.seed("/a.txt", "x\n").await;
        let r = h.call("fs.read", json!({"mount_id": MOUNT, "path": "../a.txt"})).await.unwrap();
        assert_eq!(r["content"], "1\tx");
    }

    #[tokio::test]
    async fn a_missing_required_argument_is_rejected() {
        let h = harness().await;
        let err = h.call("fs.read_lines", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT);
    }
}
