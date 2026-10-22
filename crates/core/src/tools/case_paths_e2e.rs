//! SPEC-0050 acceptance tests: a folder operation acts on that folder and what lies
//! below it, never on a folder whose name differs only by letter case. Driven
//! through the real `McpServer` tool methods over real storage (SQLite metadata,
//! local blobs); the state is read back through the volume client.
//!
//! Shared setup: project `P` owned by `o@x.com`; `/Video/x.txt` holds `upper` and
//! `/video/y.txt` holds `lower`.

use crate::mcp::server::{DeleteArgs, GetProjectQuotaArgs, McpServer, MoveArgs};
use crate::tools::admin::test_support::Fixture;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

const P: &str = "P";
const OWNER: &str = "o@x.com";

fn ok_json(r: Result<CallToolResult, rmcp::ErrorData>) -> Value {
    let r = r.expect("a tool result, not a protocol error");
    let text = r
        .content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .expect("a text content block");
    assert_ne!(r.is_error, Some(true), "expected success, got: {text}");
    serde_json::from_str(&text).expect("the tool result is JSON")
}

struct World {
    f: Fixture,
}

impl World {
    /// Project `P` with the given `(path, content)` files, parents created.
    async fn with(files: &[(&str, &str)]) -> World {
        let f = Fixture::with_config(|c| c.safety.allow_hard_delete = true).await;
        f.seed_project(P, OWNER).await;
        let w = World { f };
        for (path, content) in files {
            w.put(path, content).await;
        }
        w
    }

    /// The shared setup.
    async fn shared() -> World {
        World::with(&[("/Video/x.txt", "upper"), ("/video/y.txt", "lower")]).await
    }

    async fn put(&self, path: &str, content: &str) {
        let client = self.f.state.stores.client(P).await.unwrap();
        if let Some((parent, _)) = path.rsplit_once('/').filter(|(p, _)| !p.is_empty()) {
            client.makedirs(parent, true).await.unwrap();
        }
        client.write_text_atomic(path, content).await.unwrap();
    }

    fn server(&self) -> McpServer {
        McpServer::new(self.f.state.clone(), OWNER.to_string())
    }

    async fn delete(&self, path: &str, trash: bool) {
        let args = DeleteArgs { mount_id: P.into(), path: path.into(), recursive: true, trash };
        ok_json(self.server().fs_delete(Parameters(args)).await);
    }

    async fn mv(&self, source: &str, destination: &str) {
        let args = MoveArgs {
            mount_id: P.into(),
            source: source.into(),
            destination: destination.into(),
            overwrite: false,
        };
        ok_json(self.server().fs_move(Parameters(args)).await);
    }

    /// The text at `path`, or `None` when nothing is there.
    async fn text(&self, path: &str) -> Option<String> {
        let client = self.f.state.stores.client(P).await.unwrap();
        if client.exists(path).await.unwrap() { client.read_text(path).await.ok() } else { None }
    }

    async fn exists(&self, path: &str) -> bool {
        self.f.state.stores.client(P).await.unwrap().exists(path).await.unwrap()
    }

    /// The direct children of `/`, sorted.
    async fn root_names(&self) -> Vec<String> {
        let client = self.f.state.stores.client(P).await.unwrap();
        let mut names: Vec<String> =
            client.list_dir("/").await.unwrap().into_iter().map(|n| n.path).collect();
        names.sort();
        names
    }

    async fn used_bytes(&self) -> i64 {
        let args = GetProjectQuotaArgs { project_id: P.into() };
        ok_json(self.server().admin_get_project_quota(Parameters(args)).await)["used_bytes"]
            .as_i64()
            .expect("an integer")
    }
}

/// SPEC-0050/BT-001: a permanent delete leaves the other case alone (BR-001).
#[tokio::test]
async fn spec_0050_bt_001_a_permanent_delete_leaves_the_other_case_alone() {
    let w = World::shared().await;
    w.delete("/video", false).await;
    assert_eq!(w.root_names().await, ["/Video"]);
    assert_eq!(w.text("/Video/x.txt").await.as_deref(), Some("upper"));
    assert!(!w.exists("/video/y.txt").await, "the deleted folder's file is gone");
    assert_eq!(w.used_bytes().await, 5);
}

/// SPEC-0050/BT-020: the folder and everything below it still go (BR-001, BR-003, BR-004).
#[tokio::test]
async fn spec_0050_bt_020_the_folder_and_everything_below_it_still_go() {
    let files = [("/video/sub/z.txt", "z"), ("/video/y.txt", "y"), ("/other/o.txt", "o")];
    let w = World::with(&files).await;
    w.delete("/video", false).await;
    assert!(!w.exists("/video").await);
    assert!(!w.exists("/video/sub/z.txt").await);
    assert!(!w.exists("/video/y.txt").await);
    assert_eq!(w.text("/other/o.txt").await.as_deref(), Some("o"));

    let w = World::with(&files).await;
    w.mv("/video", "/clip").await;
    assert_eq!(w.text("/clip/sub/z.txt").await.as_deref(), Some("z"));
    assert_eq!(w.text("/clip/y.txt").await.as_deref(), Some("y"));
    assert!(!w.exists("/video").await);
}

/// SPEC-0050/BT-021: a name that only starts like the folder is not touched (BR-008).
#[tokio::test]
async fn spec_0050_bt_021_a_name_that_only_starts_like_the_folder_is_not_touched() {
    let w = World::with(&[("/video/y.txt", "y"), ("/videos/q.txt", "q")]).await;
    w.delete("/video", false).await;
    assert_eq!(w.text("/videos/q.txt").await.as_deref(), Some("q"));
    assert!(!w.exists("/video/y.txt").await);
}

/// SPEC-0050/BT-022: wildcard characters match themselves only (BR-008).
#[tokio::test]
async fn spec_0050_bt_022_wildcard_characters() {
    let w = World::with(&[
        ("/a_b/x.txt", "x"),
        ("/axb/y.txt", "y"),
        ("/50%/p.txt", "p"),
        ("/50x/q.txt", "q"),
    ])
    .await;
    w.delete("/a_b", false).await;
    w.delete("/50%", false).await;
    assert_eq!(w.text("/axb/y.txt").await.as_deref(), Some("y"));
    assert_eq!(w.text("/50x/q.txt").await.as_deref(), Some("q"));
    assert!(!w.exists("/a_b/x.txt").await);
    assert!(!w.exists("/50%/p.txt").await);
}
