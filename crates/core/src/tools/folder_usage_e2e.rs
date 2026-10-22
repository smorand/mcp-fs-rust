//! SPEC-0049 acceptance tests: `admin.get_project_folder_usage`, driven through the
//! real `McpServer` tool method (and the data plane router for the web interface)
//! over real storage. Nothing is mocked.
//!
//! Shared setup: project `Atlas` has a limit of 2 MB and stores seven files
//! (1 225 500 bytes in all); Olivia owns it, Mia is a member, Ada is a platform
//! admin, Otto belongs to nothing, Bram owns `Borealis`.

use crate::mcp::server::{GetProjectFolderUsageArgs, McpServer};
use crate::tools::admin::test_support::Fixture;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};

const ATLAS: &str = "Atlas";
const BOREALIS: &str = "Borealis";
const MIA: &str = "mia@example.com";
const OLIVIA: &str = "olivia@example.com";
const ADA: &str = "ada@example.com";
const OTTO: &str = "otto@example.com";
const BRAM: &str = "bram@example.com";
const MB: i64 = 1_048_576;
const SESSION_BUDGET: i64 = 262_144;

const FILES: [(&str, usize); 7] = [
    ("/video/demo.mp4", 600_000),
    ("/video/clips/a.mp4", 200_000),
    ("/data/export.csv", 250_000),
    ("/data/old/x.csv", 50_000),
    ("/docs/spec.docx", 120_000),
    ("/docs/notes.txt", 4_000),
    ("/readme.md", 1_500),
];

fn text_of(r: &CallToolResult) -> String {
    r.content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text content block")
}

fn ok_json(r: Result<CallToolResult, rmcp::ErrorData>) -> Value {
    let r = r.expect("a tool result, not a protocol error");
    assert_ne!(r.is_error, Some(true), "expected success, got: {}", text_of(&r));
    serde_json::from_str(&text_of(&r)).expect("the tool result is JSON")
}

fn err_text(r: Result<CallToolResult, rmcp::ErrorData>) -> String {
    let r = r.expect("a tool result, not a protocol error");
    assert_eq!(r.is_error, Some(true), "expected an error, got: {}", text_of(&r));
    text_of(&r)
}

/// Bytes that do not compress, so no test depends on content sharing by accident.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

struct World {
    f: Fixture,
}

impl World {
    /// Atlas with its limit and no file; Borealis owned by Bram.
    async fn empty() -> World {
        let f = Fixture::with_config(|c| {
            c.auth.admins = vec![ADA.to_string()];
            c.safety.write_quota_bytes = SESSION_BUDGET;
            // The purge test removes a trashed file permanently.
            c.safety.allow_hard_delete = true;
        })
        .await;
        f.seed_project(ATLAS, OLIVIA).await;
        f.seed_project(BOREALIS, BRAM).await;
        f.state.admin.add_member(ATLAS, MIA, OLIVIA).await.unwrap();
        f.state.admin.set_quota(ATLAS, Some(2 * MB)).await.unwrap();
        World { f }
    }

    /// The shared setup: Atlas with its seven files.
    async fn shared() -> World {
        let w = World::empty().await;
        for (i, (path, len)) in FILES.iter().enumerate() {
            w.seed(path, *len, i as u64).await;
        }
        w
    }

    /// Seed bytes straight into the volume: setup, not the operation tested.
    async fn seed(&self, path: &str, len: usize, seed: u64) {
        let client = self.f.state.stores.client(ATLAS).await.unwrap();
        if let Some((parent, _)) = path.rsplit_once('/').filter(|(p, _)| !p.is_empty()) {
            client.makedirs(parent, true).await.unwrap();
        }
        client.write_bytes_atomic(path, &noise(len, seed)).await.unwrap();
    }

    fn as_(&self, person: &str) -> McpServer {
        McpServer::new(self.f.state.clone(), person.to_string())
    }

    async fn read(
        &self,
        person: &str,
        project: &str,
        folder: Option<&str>,
        limit: Option<f64>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let args = GetProjectFolderUsageArgs {
            project_id: project.into(),
            folder: folder.map(String::from),
            limit,
        };
        self.as_(person).admin_get_project_folder_usage(Parameters(args)).await
    }
}

fn folder_item(path: &str, size: i64, files: i64) -> Value {
    json!({"path": path, "kind": "Folder", "size": size, "file_count": files})
}

fn file_item(path: &str, size: i64) -> Value {
    json!({"path": path, "kind": "File", "size": size})
}

/// The answer of E2E-001: the root of the shared setup.
fn root_answer() -> Value {
    json!({
        "project_id": "Atlas", "max_mb": 2, "used_bytes": 1_225_500,
        "folder": {"path": "/", "size": 1_225_500, "file_count": 7},
        "item_count": 4, "truncated": false,
        "items": [
            folder_item("/video", 800_000, 2),
            folder_item("/data", 300_000, 2),
            folder_item("/docs", 124_000, 2),
            file_item("/readme.md", 1_500),
        ],
    })
}

fn sum_of_items(out: &Value) -> i64 {
    out["items"].as_array().unwrap().iter().map(|e| e["size"].as_i64().unwrap()).sum()
}

/// SPEC-0049/E2E-001: the owner reads the root folder.
#[tokio::test]
async fn spec_0049_e2e_001_the_owner_reads_the_root_folder() {
    let w = World::shared().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None, None).await);
    assert_eq!(out, root_answer());
    assert_eq!(sum_of_items(&out), 1_225_500);
}

/// SPEC-0049/E2E-002: the admin reads the same root.
#[tokio::test]
async fn spec_0049_e2e_002_the_admin_reads_the_same_root() {
    let w = World::shared().await;
    assert_eq!(ok_json(w.read(ADA, ATLAS, None, None).await), root_answer());
}

/// SPEC-0049/E2E-003: drilling into a folder.
#[tokio::test]
async fn spec_0049_e2e_003_drilling_into_a_folder() {
    let w = World::shared().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, Some("/video"), None).await);
    assert_eq!(
        out,
        json!({
            "project_id": "Atlas", "max_mb": 2, "used_bytes": 1_225_500,
            "folder": {"path": "/video", "size": 800_000, "file_count": 2},
            "item_count": 2, "truncated": false,
            "items": [file_item("/video/demo.mp4", 600_000), folder_item("/video/clips", 200_000, 1)],
        })
    );
}

/// SPEC-0049/E2E-004: a folder of many small files outranks one bigger file.
#[tokio::test]
async fn spec_0049_e2e_004_a_folder_of_many_small_files_outranks_one_bigger_file() {
    let w = World::empty().await;
    for i in 1..=1000 {
        w.seed(&format!("/logs/l{i:04}.txt"), 100, 1).await;
    }
    w.seed("/big.bin", 90_000, 2).await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None, None).await);
    assert_eq!(out["used_bytes"], 190_000);
    assert_eq!(
        out["items"],
        json!([folder_item("/logs", 100_000, 1000), file_item("/big.bin", 90_000)])
    );
    assert_eq!(out["truncated"], false);
}

/// SPEC-0049/E2E-007: a member is refused.
#[tokio::test]
async fn spec_0049_e2e_007_a_member_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(MIA, ATLAS, None, None).await);
    assert!(e.ends_with("'mia@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

/// SPEC-0049/E2E-008: an outsider is refused.
#[tokio::test]
async fn spec_0049_e2e_008_an_outsider_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, ATLAS, None, None).await);
    assert!(e.ends_with("'otto@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

/// SPEC-0049/E2E-009: an outsider naming a missing project gets the owner refusal.
#[tokio::test]
async fn spec_0049_e2e_009_an_outsider_naming_a_missing_project_is_refused_the_same_way() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, "Ghost", None, None).await);
    assert!(e.ends_with("'otto@example.com' is not the owner of 'Ghost'"), "got: {e}");
}

/// SPEC-0049/E2E-010: the admin naming a missing project.
#[tokio::test]
async fn spec_0049_e2e_010_the_admin_naming_a_missing_project() {
    let w = World::shared().await;
    let e = err_text(w.read(ADA, "Ghost", None, None).await);
    assert!(e.ends_with("project 'Ghost' not found"), "got: {e}");
}

/// SPEC-0049/E2E-011: the owner of another project is refused.
#[tokio::test]
async fn spec_0049_e2e_011_the_owner_of_another_project_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(BRAM, ATLAS, None, None).await);
    assert!(e.ends_with("'bram@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

const CAP_REFUSAL: &str = "limit must be between 1 and 500";

/// SPEC-0049/E2E-012: a cap of 0 is refused.
#[tokio::test]
async fn spec_0049_e2e_012_a_cap_of_zero_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, None, Some(0.0)).await);
    assert!(e.ends_with(CAP_REFUSAL), "got: {e}");
}

/// SPEC-0049/E2E-013: a cap of 501 is refused.
#[tokio::test]
async fn spec_0049_e2e_013_a_cap_of_501_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, None, Some(501.0)).await);
    assert!(e.ends_with(CAP_REFUSAL), "got: {e}");
}

/// SPEC-0049/E2E-014: a negative or fractional cap is refused.
#[tokio::test]
async fn spec_0049_e2e_014_a_negative_or_fractional_cap_is_refused() {
    let w = World::shared().await;
    for bad in [-5.0, 2.5] {
        let e = err_text(w.read(OLIVIA, ATLAS, None, Some(bad)).await);
        assert!(e.ends_with(CAP_REFUSAL), "cap {bad}, got: {e}");
    }
}

/// SPEC-0049/E2E-018: a refusal reveals nothing about the files.
#[tokio::test]
async fn spec_0049_e2e_018_a_refusal_reveals_nothing_about_the_files() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, ATLAS, None, None).await);
    for secret in ["video", "readme", "800000", "1225500"] {
        assert!(!e.contains(secret), "the refusal leaks {secret}: {e}");
    }
}
