//! Search family: `fs.glob`, `fs.grep`, `fs.find_definition`, `fs.find_references`.
//!
//! Port of the C# `Tools/SearchTools.cs`. Glob and grep are engine calls; the two
//! symbol tools walk the volume with the shared `fs_ops::iter_files` and run the
//! language aware matcher from [`crate::docs::symbols`] on every file that has a
//! known language, which is what the C# `FsOps.FindDefinitions` /
//! `FsOps.FindReferences` do.
//!
//! The C# wraps unexpected exceptions of `fs.glob` into `ERR_INTERNAL_ERROR`
//! ("fs.glob failed: ..."); there is no equivalent here because the engine only
//! ever returns a typed `ToolError`, so there is nothing left to translate.

use crate::core::fs_ops;
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::tools::{norm_or, volume};

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new("fs.glob", "Find files by glob pattern, newest first (cap 100).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("pattern", "Glob pattern to match file paths, e.g. **/*.cs.")
            .opt_str("root", "/", "Absolute POSIX directory to search under.")
            .opt_flexible_str_array(
                "exclude_patterns",
                "Glob patterns whose matches are excluded from results.",
            ),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let root = norm_or(&ctx, &a, "root", "/")?;
            fs_ops::glob_files(&client, &root, &a.str("pattern")?, &a.str_array("exclude_patterns"))
                .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.grep", "Search file contents (files|content|count modes).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("pattern", "Search pattern; regex or literal depending on regex.")
            .opt_str("root", "/", "Absolute POSIX directory to search under.")
            .opt_str_null("include_glob", "Glob limiting which files are searched.")
            .opt_str_null("exclude_glob", "Glob excluding files from the search.")
            .opt_bool("regex", true, "Treat pattern as a regex when true, else a literal string.")
            .opt_bool("case_sensitive", true, "Match case sensitively.")
            .opt_str("output_mode", "content", "Output mode: files, content, or count.")
            .opt_int("context_lines", 0, "Lines of context around each match (content mode).")
            .opt_int("max_matches", 100, "Maximum number of matches to return."),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let root = norm_or(&ctx, &a, "root", "/")?;
            fs_ops::grep_files(
                &client,
                &root,
                &a.str("pattern")?,
                a.opt_str("include_glob").as_deref(),
                a.opt_str("exclude_glob").as_deref(),
                a.bool_or("regex", true),
                a.bool_or("case_sensitive", true),
                &a.str_or("output_mode", "content"),
                a.int_or("context_lines", 0),
                a.int_or("max_matches", 100),
            )
            .await
        }),
    );

    reg.add(
        ToolSchema::new("fs.find_definition", "Find a symbol definition (language-aware).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("name", "Symbol name to locate the definition of.")
            .opt_str("root", "/", "Absolute POSIX directory to search under.")
            .opt_str_null("kind", "Optional symbol kind filter, e.g. function, class, method."),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let root = norm_or(&ctx, &a, "root", "/")?;
            fs_ops::find_definitions(&client, &root, &a.str("name")?, a.opt_str("kind").as_deref()).await
        }),
    );

    reg.add(
        ToolSchema::new("fs.find_references", "Find identifier references (language-aware).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("name", "Identifier name to find references to.")
            .opt_str("root", "/", "Absolute POSIX directory to search under."),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let root = norm_or(&ctx, &a, "root", "/")?;
            fs_ops::find_references(&client, &root, &a.str("name")?).await
        }),
    );
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testkit::{MOUNT, assert_description, assert_family, assert_schema, harness};
    use serde_json::json;

    const NAMES: &[&str] =
        &["fs.glob", "fs.grep", "fs.find_definition", "fs.find_references"];

    #[test]
    fn family_registers_every_tool() {
        assert_family(register, NAMES);
    }

    /// `exclude_patterns` has NO `type` key: the C# converter accepts an array, a
    /// bare string, a comma separated string or null.
    #[test]
    fn fs_glob_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.glob",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "pattern":{"description":"Glob pattern to match file paths, e.g. **/*.cs.","type":"string"},
                 "root":{"description":"Absolute POSIX directory to search under.","type":"string","default":"/"},
                 "exclude_patterns":{"description":"Glob patterns whose matches are excluded from results.","default":null}},
               "required":["mount_id","pattern"]}"#,
        );
        assert_description(register, "fs.glob", "Find files by glob pattern, newest first (cap 100).");
    }

    #[test]
    fn fs_grep_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.grep",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "pattern":{"description":"Search pattern; regex or literal depending on regex.","type":"string"},
                 "root":{"description":"Absolute POSIX directory to search under.","type":"string","default":"/"},
                 "include_glob":{"description":"Glob limiting which files are searched.","type":"string","default":null},
                 "exclude_glob":{"description":"Glob excluding files from the search.","type":"string","default":null},
                 "regex":{"description":"Treat pattern as a regex when true, else a literal string.","type":"boolean","default":true},
                 "case_sensitive":{"description":"Match case sensitively.","type":"boolean","default":true},
                 "output_mode":{"description":"Output mode: files, content, or count.","type":"string","default":"content"},
                 "context_lines":{"description":"Lines of context around each match (content mode).","type":"integer","default":0},
                 "max_matches":{"description":"Maximum number of matches to return.","type":"integer","default":100}},
               "required":["mount_id","pattern"]}"#,
        );
    }

    #[test]
    fn fs_find_definition_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.find_definition",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "name":{"description":"Symbol name to locate the definition of.","type":"string"},
                 "root":{"description":"Absolute POSIX directory to search under.","type":"string","default":"/"},
                 "kind":{"description":"Optional symbol kind filter, e.g. function, class, method.","type":"string","default":null}},
               "required":["mount_id","name"]}"#,
        );
    }

    #[tokio::test]
    async fn glob_matches_by_pattern() {
        let h = harness().await;
        h.seed("/src/app.py", "print(1)\n").await;
        h.seed("/README.md", "# hi\n").await;
        let r = h.call("fs.glob", json!({"mount_id": MOUNT, "pattern": "*.py"})).await.unwrap();
        assert_eq!(r["matches"], json!(["/src/app.py"]));
        assert_eq!(r["truncated"], false);
    }

    /// The tolerant array shape: a bare comma separated string is accepted.
    #[tokio::test]
    async fn glob_accepts_a_csv_exclude_patterns() {
        let h = harness().await;
        h.seed("/src/app.py", "x\n").await;
        h.seed("/src/test_app.py", "x\n").await;
        let r = h
            .call(
                "fs.glob",
                json!({"mount_id": MOUNT, "pattern": "*.py", "exclude_patterns": "*/test_*.py"}),
            )
            .await
            .unwrap();
        assert_eq!(r["matches"], json!(["/src/app.py"]));
    }

    #[tokio::test]
    async fn grep_supports_the_three_output_modes() {
        let h = harness().await;
        h.seed("/src/app.py", "def hello(name):\n    total = 1\n    return total\n").await;
        let content = h.call("fs.grep", json!({"mount_id": MOUNT, "pattern": "total"})).await.unwrap();
        assert_eq!(content["matches"].as_array().unwrap().len(), 2);

        let files = h
            .call("fs.grep", json!({"mount_id": MOUNT, "pattern": "total", "output_mode": "files"}))
            .await
            .unwrap();
        assert_eq!(files["files"], json!(["/src/app.py"]));

        let count = h
            .call("fs.grep", json!({"mount_id": MOUNT, "pattern": "total", "output_mode": "count"}))
            .await
            .unwrap();
        assert_eq!(count["count"], 2);
        assert_eq!(count["files"], 1);
    }

    #[tokio::test]
    async fn grep_rejects_an_invalid_regex() {
        let h = harness().await;
        h.seed("/a.txt", "x\n").await;
        let err = h.call("fs.grep", json!({"mount_id": MOUNT, "pattern": "("})).await.unwrap_err();
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn find_definition_and_references_are_language_aware() {
        let h = harness().await;
        h.seed("/src/app.py", "def hello(name):\n    total = 1\n    return total\n").await;
        h.seed("/notes.txt", "hello lives here\n").await;

        let defs = h
            .call("fs.find_definition", json!({"mount_id": MOUNT, "name": "hello"}))
            .await
            .unwrap();
        let list = defs["definitions"].as_array().unwrap();
        assert_eq!(list.len(), 1, "the .txt file has no language and is skipped");
        assert_eq!(list[0]["path"], "/src/app.py");
        assert_eq!(list[0]["kind"], "function_definition");
        assert_eq!(list[0]["line"], 1);

        let refs = h
            .call("fs.find_references", json!({"mount_id": MOUNT, "name": "total"}))
            .await
            .unwrap();
        let hits = refs["references"].as_array().unwrap();
        assert!(!hits.is_empty());
        // The reference payload carries no `name`, matching the C# shape.
        assert!(hits[0].get("name").is_none());
    }

    #[tokio::test]
    async fn find_references_requires_a_name() {
        let h = harness().await;
        let err = h
            .call("fs.find_references", json!({"mount_id": MOUNT, "name": ""}))
            .await
            .unwrap_err();
        assert_eq!(err.code, crate::errors::code::INVALID_ARGUMENT);
        assert_eq!(err.message, "name is required");
    }

    #[tokio::test]
    async fn find_definition_filters_by_kind() {
        let h = harness().await;
        h.seed("/src/lib.rs", "struct Node;\nfn make() {}\n").await;
        let structs = h
            .call("fs.find_definition", json!({"mount_id": MOUNT, "name": "", "kind": "struct"}))
            .await
            .unwrap();
        let list = structs["definitions"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["name"], "Node");
    }
}
