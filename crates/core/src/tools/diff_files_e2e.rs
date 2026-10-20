//! SPEC-0045 US-0055 acceptance tests: `fs.diff_files`, driven through the real
//! `McpServer::fs_diff_files` handler over real storage (SQLite metadata, local
//! blobs). Nothing is mocked.

use crate::mcp::server::{DiffFilesArgs, McpServer};
use crate::storage::traits::AdminBackend;
use crate::tools::admin::test_support::Fixture;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};

const OWNER: &str = "owner@test.com";
const ALICE: &str = "alice@example.com";
const BOB: &str = "bob@example.com";
const CAROL: &str = "carol@example.com";

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

/// The common data: alpha and beta, Alice in both, Bob in alpha only, and the
/// two notes of alpha.
async fn world() -> Fixture {
    let f = Fixture::new().await;
    f.seed_project("alpha", OWNER).await;
    f.seed_project("beta", OWNER).await;
    f.state.admin.add_member("alpha", ALICE, OWNER).await.unwrap();
    f.state.admin.add_member("beta", ALICE, OWNER).await.unwrap();
    f.state.admin.add_member("alpha", BOB, OWNER).await.unwrap();
    put(&f, "alpha", "/notes/a.txt", "one\ntwo\nthree\n").await;
    put(&f, "alpha", "/notes/b.txt", "one\n2\nthree\n").await;
    f
}

async fn put(f: &Fixture, project: &str, path: &str, text: &str) {
    f.state.stores.client(project).await.unwrap().write_text_atomic(path, text).await.unwrap();
}

async fn diff(
    f: &Fixture,
    person: &str,
    a: (&str, &str),
    b: (&str, &str),
) -> Result<CallToolResult, rmcp::ErrorData> {
    let server = McpServer::new(f.state.clone(), person.to_string());
    server
        .fs_diff_files(Parameters(DiffFilesArgs {
            mount_id_a: a.0.to_string(),
            path_a: a.1.to_string(),
            mount_id_b: b.0.to_string(),
            path_b: b.1.to_string(),
        }))
        .await
}

/// SPEC-0045/E2E-001: two files of one project differ on one line.
#[tokio::test]
async fn spec_0045_e2e_001_two_files_of_one_project() {
    let f = world().await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/b.txt")).await);
    assert_eq!(
        v,
        json!({
            "identical": false,
            "diff": "--- alpha:/notes/a.txt\n+++ alpha:/notes/b.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+2\n three\n"
        })
    );
}

/// SPEC-0045/E2E-002: two projects.
#[tokio::test]
async fn spec_0045_e2e_002_two_projects() {
    let f = world().await;
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    put(&f, "beta", "/cfg/app.txt", "port=8080\n").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    assert_eq!(v["identical"], false);
    assert_eq!(
        v["diff"],
        "--- alpha:/cfg/app.txt\n+++ beta:/cfg/app.txt\n@@ -1 +1 @@\n-port=80\n+port=8080\n"
    );
}

/// SPEC-0045/E2E-003: identical files.
#[tokio::test]
async fn spec_0045_e2e_003_identical_files() {
    let f = world().await;
    put(&f, "alpha", "/notes/copy.txt", "one\ntwo\nthree\n").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/copy.txt")).await);
    assert_eq!(v, json!({"identical": true, "diff": ""}));
}

/// SPEC-0045/E2E-107: no access to the second project, the answer is the same
/// whether the file exists or not.
#[tokio::test]
async fn spec_0045_e2e_107_no_access_to_the_second_project() {
    let f = world().await;
    put(&f, "beta", "/cfg/app.txt", "secret=1\n").await;
    let e1 = err_text(diff(&f, BOB, ("alpha", "/notes/a.txt"), ("beta", "/cfg/app.txt")).await);
    assert!(
        e1.ends_with("ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'"),
        "got: {e1}"
    );
    for leak in ["secret", "one", "two", "three"] {
        assert!(!e1.contains(leak), "leaked {leak}: {e1}");
    }
    let e2 = err_text(diff(&f, BOB, ("alpha", "/notes/a.txt"), ("beta", "/cfg/nothing.txt")).await);
    assert_eq!(e1, e2);
}

/// SPEC-0045/E2E-108: no access to the first project.
#[tokio::test]
async fn spec_0045_e2e_108_no_access_to_the_first_project() {
    let f = world().await;
    put(&f, "beta", "/cfg/app.txt", "secret=1\n").await;
    let e = err_text(diff(&f, BOB, ("beta", "/cfg/app.txt"), ("alpha", "/notes/a.txt")).await);
    assert!(e.ends_with("ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'"), "got: {e}");
    for leak in ["secret", "one", "two", "three"] {
        assert!(!e.contains(leak), "leaked {leak}: {e}");
    }
}

/// SPEC-0045/E2E-201: empty against non empty.
#[tokio::test]
async fn spec_0045_e2e_201_empty_against_non_empty() {
    let f = world().await;
    put(&f, "alpha", "/e.txt", "").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/e.txt"), ("alpha", "/notes/b.txt")).await);
    assert_eq!(v["identical"], false);
    assert_eq!(
        v["diff"],
        "--- alpha:/e.txt\n+++ alpha:/notes/b.txt\n@@ -0,0 +1,3 @@\n+one\n+2\n+three\n"
    );
}

/// SPEC-0045/E2E-202: two empty files.
#[tokio::test]
async fn spec_0045_e2e_202_two_empty_files() {
    let f = world().await;
    put(&f, "alpha", "/e1.txt", "").await;
    put(&f, "alpha", "/e2.txt", "").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/e1.txt"), ("alpha", "/e2.txt")).await);
    assert_eq!(v, json!({"identical": true, "diff": ""}));
}

/// SPEC-0045/E2E-203: the same file on both sides changes nothing.
#[tokio::test]
async fn spec_0045_e2e_203_the_same_file_on_both_sides() {
    let f = world().await;
    let client = f.state.stores.client("alpha").await.unwrap();
    let before_text = client.read_text("/notes/a.txt").await.unwrap();
    let before_used = crate::quota::used_bytes(&client).await.unwrap();
    let v = ok_json(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/a.txt")).await);
    assert_eq!(v, json!({"identical": true, "diff": ""}));
    assert_eq!(client.read_text("/notes/a.txt").await.unwrap(), before_text);
    assert_eq!(crate::quota::used_bytes(&client).await.unwrap(), before_used);
}

/// SPEC-0045/E2E-206: Windows against Unix line endings.
#[tokio::test]
async fn spec_0045_e2e_206_windows_against_unix_line_endings() {
    let f = world().await;
    put(&f, "alpha", "/w.txt", "one\r\ntwo\r\n").await;
    put(&f, "alpha", "/u.txt", "one\ntwo\n").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/w.txt"), ("alpha", "/u.txt")).await);
    assert_eq!(v["identical"], false);
    let d = v["diff"].as_str().unwrap();
    let body: Vec<&str> = d.lines().skip(2).collect();
    assert_eq!(body.iter().filter(|l| l.starts_with('-')).count(), 2, "got: {d}");
    assert_eq!(body.iter().filter(|l| l.starts_with('+')).count(), 2, "got: {d}");
}

/// SPEC-0045/E2E-207: no final line ending, and no "no newline" marker.
#[tokio::test]
async fn spec_0045_e2e_207_no_final_line_ending() {
    let f = world().await;
    put(&f, "alpha", "/n1.txt", "a").await;
    put(&f, "alpha", "/n2.txt", "b").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/n1.txt"), ("alpha", "/n2.txt")).await);
    assert_eq!(v["identical"], false);
    assert_eq!(v["diff"], "--- alpha:/n1.txt\n+++ alpha:/n2.txt\n@@ -1 +1 @@\n-a\n+b\n");
}

/// SPEC-0045/E2E-208: two distant changes make two blocks.
#[tokio::test]
async fn spec_0045_e2e_208_two_distant_changes() {
    let f = world().await;
    let old: String = (1..=20).map(|i| format!("line{i}\n")).collect();
    let new = old.replacen("line1\n", "LINE1\n", 1).replacen("line20\n", "LINE20\n", 1);
    put(&f, "alpha", "/l1.txt", &old).await;
    put(&f, "alpha", "/l2.txt", &new).await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/l1.txt"), ("alpha", "/l2.txt")).await);
    assert_eq!(
        v["diff"],
        "--- alpha:/l1.txt\n+++ alpha:/l2.txt\n\
         @@ -1,4 +1,4 @@\n-line1\n+LINE1\n line2\n line3\n line4\n\
         @@ -17,4 +17,4 @@\n line17\n line18\n line19\n-line20\n+LINE20\n"
    );
}

/// SPEC-0045/E2E-209: accents and emoji survive in headers and lines.
#[tokio::test]
async fn spec_0045_e2e_209_accents_and_emoji() {
    let f = world().await;
    put(&f, "alpha", "/é/a.txt", "café ☕\n").await;
    put(&f, "alpha", "/é/b.txt", "café 🍵\n").await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/é/a.txt"), ("alpha", "/é/b.txt")).await);
    assert_eq!(
        v["diff"],
        "--- alpha:/é/a.txt\n+++ alpha:/é/b.txt\n@@ -1 +1 @@\n-café ☕\n+café 🍵\n"
    );
}

/// SPEC-0045/E2E-210: more than 2000 lines, no truncation.
#[tokio::test]
async fn spec_0045_e2e_210_more_than_2000_lines() {
    let f = world().await;
    let old: String = (1..=3000).map(|i| format!("row{i}\n")).collect();
    let new = old.replacen("row2999\n", "CHANGED\n", 1);
    put(&f, "alpha", "/big1.txt", &old).await;
    put(&f, "alpha", "/big2.txt", &new).await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/big1.txt"), ("alpha", "/big2.txt")).await);
    assert_eq!(
        v["diff"],
        "--- alpha:/big1.txt\n+++ alpha:/big2.txt\n@@ -2996,5 +2996,5 @@\n row2996\n row2997\n row2998\n-row2999\n+CHANGED\n row3000\n"
    );
}

/// SPEC-0045/E2E-211: a plain member (not owner) of both projects.
#[tokio::test]
async fn spec_0045_e2e_211_a_plain_member() {
    let f = world().await;
    f.state.admin.add_member("alpha", CAROL, OWNER).await.unwrap();
    f.state.admin.add_member("beta", CAROL, OWNER).await.unwrap();
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    put(&f, "beta", "/cfg/app.txt", "port=8080\n").await;
    let carol = ok_json(diff(&f, CAROL, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    let alice = ok_json(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    assert_eq!(carol, alice);
    assert_eq!(carol["identical"], false);
}

/// SPEC-0045/E2E-225: doubled separators are normalized, in errors and headers.
#[tokio::test]
async fn spec_0045_e2e_225_path_with_doubled_separators() {
    let f = world().await;
    let e = err_text(
        diff(&f, ALICE, ("alpha", "/notes//a.txt"), ("alpha", "/notes/missing.txt")).await,
    );
    assert!(e.ends_with("ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found"), "got: {e}");
    let v = ok_json(diff(&f, ALICE, ("alpha", "/notes//a.txt"), ("alpha", "/notes/b.txt")).await);
    assert!(v["diff"].as_str().unwrap().starts_with("--- alpha:/notes/a.txt\n"), "got: {v}");
}

/// SPEC-0045/E2E-101: first file missing, named with its project.
#[tokio::test]
async fn spec_0045_e2e_101_first_file_missing() {
    let f = world().await;
    let e =
        err_text(diff(&f, ALICE, ("alpha", "/notes/missing.txt"), ("alpha", "/notes/b.txt")).await);
    assert!(e.ends_with("ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found"), "got: {e}");
    assert!(!e.contains("identical") && !e.contains("@@"), "got: {e}");
}

/// SPEC-0045/E2E-102: second file missing, the second one is named.
#[tokio::test]
async fn spec_0045_e2e_102_second_file_missing() {
    let f = world().await;
    let e =
        err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/missing.txt")).await);
    assert!(e.ends_with("ERR_NOT_FOUND: 'alpha:/notes/missing.txt' not found"), "got: {e}");
    assert!(!e.contains("identical") && !e.contains("@@"), "got: {e}");
}

/// SPEC-0045/E2E-103: a directory is refused, named with its project.
#[tokio::test]
async fn spec_0045_e2e_103_a_directory() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes"), ("alpha", "/notes/b.txt")).await);
    assert!(e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory"), "got: {e}");
}

/// SPEC-0045/E2E-109: deleted project (green on arrival, inherited).
#[tokio::test]
async fn spec_0045_e2e_109_deleted_project() {
    let f = world().await;
    put(&f, "beta", "/cfg/app.txt", "x\n").await;
    f.admin_store.soft_delete_project("beta").await.unwrap();
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("beta", "/cfg/app.txt")).await);
    assert!(e.ends_with("ERR_PROJECT_NOT_FOUND: project 'beta' not found"), "got: {e}");
}

/// SPEC-0045/E2E-110: access outranks the directory problem, which outranks
/// the missing second file.
#[tokio::test]
async fn spec_0045_e2e_110_several_problems_fixed_order() {
    let f = world().await;
    let bob = err_text(diff(&f, BOB, ("alpha", "/notes"), ("beta", "/cfg/nothing.txt")).await);
    assert!(
        bob.ends_with("ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'"),
        "got: {bob}"
    );
    let alice = err_text(diff(&f, ALICE, ("alpha", "/notes"), ("beta", "/cfg/nothing.txt")).await);
    assert!(alice.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory"), "got: {alice}");
}

/// SPEC-0045/E2E-111: climbing above the top stops at the top.
#[tokio::test]
async fn spec_0045_e2e_111_climbing_above_the_top() {
    let f = world().await;
    let e = err_text(
        diff(&f, ALICE, ("alpha", "/notes/../../etc/passwd"), ("alpha", "/notes/b.txt")).await,
    );
    assert!(e.ends_with("ERR_NOT_FOUND: 'alpha:/etc/passwd' not found"), "got: {e}");
}

/// SPEC-0045/E2E-212: the second path is a directory.
#[tokio::test]
async fn spec_0045_e2e_212_second_path_is_a_directory() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes")).await);
    assert!(e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/notes' is a directory"), "got: {e}");
    assert!(!e.contains("@@"), "got: {e}");
}

/// SPEC-0045/E2E-217: first file in a deleted project (green on arrival).
#[tokio::test]
async fn spec_0045_e2e_217_first_file_in_a_deleted_project() {
    let f = world().await;
    put(&f, "beta", "/cfg/app.txt", "x\n").await;
    f.admin_store.soft_delete_project("beta").await.unwrap();
    let e = err_text(diff(&f, ALICE, ("beta", "/cfg/app.txt"), ("alpha", "/notes/a.txt")).await);
    assert!(e.ends_with("ERR_PROJECT_NOT_FOUND: project 'beta' not found"), "got: {e}");
}

/// SPEC-0045/E2E-218: a deleted project brought back compares as before
/// (green on arrival).
#[tokio::test]
async fn spec_0045_e2e_218_deleted_project_brought_back() {
    let f = world().await;
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    put(&f, "beta", "/cfg/app.txt", "port=8080\n").await;
    f.admin_store.soft_delete_project("beta").await.unwrap();
    f.state.admin.undelete_project("beta").await.unwrap();
    let v = ok_json(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/app.txt")).await);
    assert_eq!(
        v["diff"],
        "--- alpha:/cfg/app.txt\n+++ beta:/cfg/app.txt\n@@ -1 +1 @@\n-port=80\n+port=8080\n"
    );
}

/// SPEC-0045/E2E-219: missing second file in another project.
#[tokio::test]
async fn spec_0045_e2e_219_missing_second_file_in_another_project() {
    let f = world().await;
    put(&f, "alpha", "/cfg/app.txt", "port=80\n").await;
    let e =
        err_text(diff(&f, ALICE, ("alpha", "/cfg/app.txt"), ("beta", "/cfg/missing.txt")).await);
    assert!(e.ends_with("ERR_NOT_FOUND: 'beta:/cfg/missing.txt' not found"), "got: {e}");
}

/// SPEC-0045/E2E-222: climbing above the top, second file, other project.
#[tokio::test]
async fn spec_0045_e2e_222_climbing_above_the_top_second_file() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("beta", "/../secret.txt")).await);
    assert!(e.ends_with("ERR_NOT_FOUND: 'beta:/secret.txt' not found"), "got: {e}");
}

/// SPEC-0045/E2E-226: path normalization does not outrank access (green on
/// arrival).
#[tokio::test]
async fn spec_0045_e2e_226_normalization_does_not_outrank_access() {
    let f = world().await;
    let e = err_text(diff(&f, BOB, ("alpha", "/notes/a.txt"), ("beta", "/../x.txt")).await);
    assert!(e.ends_with("ERR_FORBIDDEN: 'bob@example.com' is not a member of 'beta'"), "got: {e}");
}

/// SPEC-0045/E2E-228: a NUL in a path is refused before any lookup (green on
/// arrival).
#[tokio::test]
async fn spec_0045_e2e_228_path_with_a_nul_character() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/notes/b.txt\0")).await);
    assert!(e.ends_with("ERR_PATH_OUT_OF_BOUNDS: path contains a NUL byte"), "got: {e}");
}

/// SPEC-0045/E2E-229: an empty path means the root, which is a directory.
#[tokio::test]
async fn spec_0045_e2e_229_empty_path() {
    let f = world().await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "")).await);
    assert!(e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/' is a directory"), "got: {e}");
}

async fn put_bytes(f: &Fixture, project: &str, path: &str, data: &[u8]) {
    let client = f.state.stores.client(project).await.unwrap();
    client.write_bytes_atomic(path, data).await.unwrap();
}

const OVER_LIMIT: &str = "ERR_INVALID_ARGUMENT: 'alpha:/logs/big.log' is 5242881 bytes, over the diff limit of 5242880 bytes";

/// SPEC-0045/E2E-104: a NUL byte makes the file not text.
#[tokio::test]
async fn spec_0045_e2e_104_a_file_with_a_nul_byte() {
    let f = world().await;
    put_bytes(&f, "alpha", "/img/logo.png", &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x00])
        .await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/img/logo.png")).await);
    assert!(
        e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/img/logo.png' is not a text file"),
        "got: {e}"
    );
    assert!(!e.contains("@@"), "got: {e}");
}

/// SPEC-0045/E2E-105: invalid UTF-8 without a NUL.
#[tokio::test]
async fn spec_0045_e2e_105_invalid_utf8() {
    let f = world().await;
    put_bytes(&f, "alpha", "/data/latin1.txt", &[0x63, 0x61, 0x66, 0xE9]).await;
    let e =
        err_text(diff(&f, ALICE, ("alpha", "/data/latin1.txt"), ("alpha", "/notes/a.txt")).await);
    assert!(
        e.ends_with("ERR_INVALID_ARGUMENT: 'alpha:/data/latin1.txt' is not a text file"),
        "got: {e}"
    );
}

/// SPEC-0045/E2E-106: over the default limit, then fine after the operator
/// raises it and restarts (a new fixture models the restart).
#[tokio::test]
async fn spec_0045_e2e_106_a_file_over_the_limit() {
    let f = world().await;
    let big = vec![b'a'; 5_242_881];
    put_bytes(&f, "alpha", "/logs/big.log", &big).await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/logs/big.log")).await);
    assert!(e.ends_with(OVER_LIMIT), "got: {e}");

    let g = Fixture::with_config(|c| {
        c.safety.max_diff_file_bytes = crate::config::DiffLimit::new(6_291_456)
    })
    .await;
    g.seed_project("alpha", OWNER).await;
    g.state.admin.add_member("alpha", ALICE, OWNER).await.unwrap();
    put(&g, "alpha", "/notes/a.txt", "one\ntwo\nthree\n").await;
    put_bytes(&g, "alpha", "/logs/big.log", &big).await;
    let v = ok_json(diff(&g, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/logs/big.log")).await);
    assert_eq!(v["identical"], false);
}

/// SPEC-0045/E2E-204: exactly at the limit is accepted.
#[tokio::test]
async fn spec_0045_e2e_204_exactly_at_the_limit() {
    let f = world().await;
    put_bytes(&f, "alpha", "/logs/edge.log", &vec![b'a'; 5_242_880]).await;
    let v = ok_json(diff(&f, ALICE, ("alpha", "/logs/edge.log"), ("alpha", "/notes/a.txt")).await);
    assert_eq!(v["identical"], false);
}

/// SPEC-0045/E2E-205: one byte over the limit is refused.
#[tokio::test]
async fn spec_0045_e2e_205_one_byte_over_the_limit() {
    let f = world().await;
    put_bytes(&f, "alpha", "/logs/over.log", &vec![b'a'; 5_242_881]).await;
    let e = err_text(diff(&f, ALICE, ("alpha", "/notes/a.txt"), ("alpha", "/logs/over.log")).await);
    assert!(
        e.ends_with(
            "ERR_INVALID_ARGUMENT: 'alpha:/logs/over.log' is 5242881 bytes, over the diff limit of 5242880 bytes"
        ),
        "got: {e}"
    );
}
