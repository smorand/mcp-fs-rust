//! SPEC-0047 acceptance tests: `admin.get_project_disk_usage`, driven through the
//! real `McpServer` tool method over real storage (SQLite metadata, local
//! blobs). Nothing is mocked.
//!
//! Shared setup: project `Atlas` has a limit of 2 MB and stores five files
//! (975 500 bytes in all); Olivia owns it, Mia is a member, Ada is a platform
//! admin, Otto belongs to nothing, Bram owns `Borealis`.

use crate::mcp::server::{GetProjectDiskUsageArgs, McpServer};
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

const FILES: [(&str, usize); 5] = [
    ("/video/demo.mp4", 600_000),
    ("/data/export.csv", 250_000),
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
        })
        .await;
        f.seed_project(ATLAS, OLIVIA).await;
        f.seed_project(BOREALIS, BRAM).await;
        f.state.admin.add_member(ATLAS, MIA, OLIVIA).await.unwrap();
        f.state.admin.set_quota(ATLAS, Some(2 * MB)).await.unwrap();
        World { f }
    }

    /// The shared setup: Atlas with its five files.
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
        limit: Option<f64>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let args = GetProjectDiskUsageArgs { project_id: project.into(), limit };
        self.as_(person).admin_get_project_disk_usage(Parameters(args)).await
    }
}

fn entry(path: &str, size: i64) -> Value {
    json!({"path": path, "size": size, "state": "Active", "original_path": null})
}

/// The standard answer of the spec: five entries, each `Active`.
fn standard() -> Value {
    json!({
        "project_id": "Atlas",
        "max_mb": 2,
        "used_bytes": 975_500,
        "file_count": 5,
        "truncated": false,
        "entries": [
            entry("/video/demo.mp4", 600_000),
            entry("/data/export.csv", 250_000),
            entry("/docs/spec.docx", 120_000),
            entry("/docs/notes.txt", 4_000),
            entry("/readme.md", 1_500),
        ],
    })
}

/// SPEC-0047/E2E-001: the owner reads the breakdown (FR-NEW-001, 002, 003, 008).
#[tokio::test]
async fn spec_0047_e2e_001_the_owner_reads_the_breakdown() {
    let w = World::shared().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(out, standard());
    let sum: i64 =
        out["entries"].as_array().unwrap().iter().map(|e| e["size"].as_i64().unwrap()).sum();
    assert_eq!(sum, out["used_bytes"].as_i64().unwrap());
}

/// SPEC-0047/E2E-002: the admin reads the same breakdown (FR-NEW-001, 007).
#[tokio::test]
async fn spec_0047_e2e_002_the_admin_reads_the_same_breakdown() {
    let w = World::shared().await;
    assert_eq!(ok_json(w.read(ADA, ATLAS, None).await), standard());
}

/// SPEC-0047/E2E-007: a member is refused (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_007_a_member_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(MIA, ATLAS, None).await);
    assert!(e.ends_with("'mia@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

/// SPEC-0047/E2E-008: an outsider is refused (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_008_an_outsider_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, ATLAS, None).await);
    assert!(e.ends_with("'otto@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

/// SPEC-0047/E2E-009: an outsider naming a missing project gets the owner
/// refusal, so existence cannot be probed (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_009_an_outsider_naming_a_missing_project_is_refused_the_same_way() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, "Ghost", None).await);
    assert!(e.ends_with("'otto@example.com' is not the owner of 'Ghost'"), "got: {e}");
}

/// SPEC-0047/E2E-010: the admin naming a missing project (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_010_the_admin_naming_a_missing_project() {
    let w = World::shared().await;
    let e = err_text(w.read(ADA, "Ghost", None).await);
    assert!(e.ends_with("project 'Ghost' not found"), "got: {e}");
}

/// SPEC-0047/E2E-011: the owner of another project is refused (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_011_the_owner_of_another_project_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(BRAM, ATLAS, None).await);
    assert!(e.ends_with("'bram@example.com' is not the owner of 'Atlas'"), "got: {e}");
}

const CAP_REFUSAL: &str = "limit must be between 1 and 500";

/// SPEC-0047/E2E-012: a cap of 0 is refused (FR-NEW-004).
#[tokio::test]
async fn spec_0047_e2e_012_a_cap_of_zero_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, Some(0.0)).await);
    assert!(e.ends_with(CAP_REFUSAL), "got: {e}");
}

/// SPEC-0047/E2E-013: a cap of 501 is refused (FR-NEW-004).
#[tokio::test]
async fn spec_0047_e2e_013_a_cap_of_501_is_refused() {
    let w = World::shared().await;
    let e = err_text(w.read(OLIVIA, ATLAS, Some(501.0)).await);
    assert!(e.ends_with(CAP_REFUSAL), "got: {e}");
}

/// SPEC-0047/E2E-014: a negative or fractional cap is refused (FR-NEW-004).
#[tokio::test]
async fn spec_0047_e2e_014_a_negative_or_fractional_cap_is_refused() {
    let w = World::shared().await;
    for bad in [-5.0, 2.5] {
        let e = err_text(w.read(OLIVIA, ATLAS, Some(bad)).await);
        assert!(e.ends_with(CAP_REFUSAL), "cap {bad}, got: {e}");
    }
}

/// SPEC-0047/E2E-016: a refusal reveals nothing about the files (FR-NEW-007).
#[tokio::test]
async fn spec_0047_e2e_016_a_refusal_reveals_nothing_about_the_files() {
    let w = World::shared().await;
    let e = err_text(w.read(OTTO, ATLAS, None).await);
    for secret in ["demo", "export", "600000", "975500", "readme"] {
        assert!(!e.contains(secret), "the refusal leaks {secret}: {e}");
    }
}

/// SPEC-0047/E2E-022: an empty project (FR-NEW-001, 003, 005).
#[tokio::test]
async fn spec_0047_e2e_022_an_empty_project() {
    let w = World::empty().await;
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(
        out,
        json!({"project_id": "Atlas", "max_mb": 2, "used_bytes": 0, "file_count": 0,
               "truncated": false, "entries": []})
    );
}

/// SPEC-0047/E2E-023: the default cap is 100 (FR-NEW-004, 005).
#[tokio::test]
async fn spec_0047_e2e_023_the_default_cap_is_100() {
    let w = World::empty().await;
    for i in 0..120 {
        w.seed(&format!("/f{i:03}.bin"), 1_000, i).await;
    }
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    assert_eq!(out["entries"].as_array().unwrap().len(), 100);
    assert_eq!(out["file_count"], 120);
    assert_eq!(out["used_bytes"], 120_000);
    assert_eq!(out["truncated"], true);
}

/// SPEC-0047/E2E-027: equal sizes are ordered by path in character code order,
/// so an uppercase name comes before a lowercase one (FR-NEW-002).
#[tokio::test]
async fn spec_0047_e2e_027_equal_sizes_are_ordered_by_path() {
    let w = World::empty().await;
    for (i, p) in ["/b.txt", "/a.txt", "/C.txt"].iter().enumerate() {
        w.seed(p, 10, i as u64).await;
    }
    let out = ok_json(w.read(OLIVIA, ATLAS, None).await);
    let paths: Vec<&str> =
        out["entries"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["/C.txt", "/a.txt", "/b.txt"]);
}
