//! Metadata family: `fs.stat`, `fs.exists`, `fs.hash`.
//!
//! Port of the C# `Tools/MetadataTools.cs`. None of the three mutates, so none
//! records a read: they are pure probes.

use crate::core::fs_ops;
use crate::mcp::ToolSchema;
use crate::mcp::registry::{ToolRegistry, handler};
use crate::tools::{norm, volume};

pub fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new("fs.stat", "POSIX metadata for a path.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume."),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::stat_info(&client, &path).await
        }),
    );

    reg.add(
        ToolSchema::new("fs.exists", "Probe whether a path exists and its kind.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path to probe."),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::exists_info(&client, &path).await
        }),
    );

    reg.add(
        ToolSchema::new("fs.hash", "Content hash (md5|sha1|sha256|sha512).")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path within the volume.")
            .opt_str("algo", "sha256", "Hash algorithm: md5, sha1, sha256, or sha512."),
        handler(|ctx, a| async move {
            let (_mount, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;
            fs_ops::hash_file(&client, &path, &a.str_or("algo", "sha256")).await
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::code;
    use crate::tools::testkit::{MOUNT, assert_description, assert_family, assert_schema, harness};
    use serde_json::json;

    const NAMES: &[&str] = &["fs.stat", "fs.exists", "fs.hash"];

    #[test]
    fn family_registers_every_tool() {
        assert_family(register, NAMES);
    }

    #[test]
    fn fs_hash_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.hash",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"},
                 "algo":{"description":"Hash algorithm: md5, sha1, sha256, or sha512.","type":"string","default":"sha256"}},
               "required":["mount_id","path"]}"#,
        );
        assert_description(register, "fs.hash", "Content hash (md5|sha1|sha256|sha512).");
    }

    #[test]
    fn fs_exists_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.exists",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path to probe.","type":"string"}},
               "required":["mount_id","path"]}"#,
        );
    }

    #[test]
    fn fs_stat_schema_matches_the_contract() {
        assert_schema(
            register,
            "fs.stat",
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "path":{"description":"Absolute POSIX path within the volume.","type":"string"}},
               "required":["mount_id","path"]}"#,
        );
    }

    #[tokio::test]
    async fn stat_reports_the_synthetic_posix_metadata() {
        let h = harness().await;
        h.seed("/a.txt", "hello world\n").await;
        let r = h.call("fs.stat", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        assert_eq!(r["path"], "/a.txt");
        assert_eq!(r["size"], 12);
        assert_eq!(r["mode"], "0o644");
        assert_eq!(r["kind"], "file");
        assert_eq!(r["uid"], 1000);
        assert_eq!(r["gid"], 1000);
    }

    #[tokio::test]
    async fn exists_reports_kind_or_null() {
        let h = harness().await;
        h.seed("/a.txt", "x\n").await;
        let file = h.call("fs.exists", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        assert_eq!(file, json!({"exists": true, "kind": "file"}));
        let missing = h.call("fs.exists", json!({"mount_id": MOUNT, "path": "/nope"})).await.unwrap();
        assert_eq!(missing, json!({"exists": false, "kind": null}));
    }

    #[tokio::test]
    async fn hash_defaults_to_sha256_and_rejects_an_unknown_algo() {
        let h = harness().await;
        h.seed("/a.txt", "hello world\n").await;
        let r = h.call("fs.hash", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        assert_eq!(r["algo"], "sha256");
        // sha256 of "hello world\n", the same value `shasum -a 256` reports.
        assert_eq!(
            r["hash"],
            "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447"
        );
        assert_eq!(r["size"], 12);

        let md5 = h
            .call("fs.hash", json!({"mount_id": MOUNT, "path": "/a.txt", "algo": "md5"}))
            .await
            .unwrap();
        assert_eq!(md5["hash"].as_str().unwrap().len(), 32);

        let err = h
            .call("fs.hash", json!({"mount_id": MOUNT, "path": "/a.txt", "algo": "crc32"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn stat_on_a_missing_path_is_not_found() {
        let h = harness().await;
        let err = h.call("fs.stat", json!({"mount_id": MOUNT, "path": "/nope"})).await.unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }
}
