//! US-025 acceptance tests: `git.pr_create` and the normalized pull request
//! model. Every test runs offline: the provider is a [`MockProviderApi`]
//! implementing [`ProviderClient`] and returning canned JSON.
//!
//! Scope note. This story ships `git.pr_create` and the model; `git.pr_get` and
//! `git.pr_list` belong to a later story. The acceptance tests written against
//! `pr_get` (E2E-NEW-709, 735, 736, 737, 793, 798, 926) therefore exercise the
//! same normalization contract at the only surface that exists here: the
//! mapper for 709/735/736/737/798, and the real `git.pr_create` response for
//! 793 and 926. Each of those tests says what it substituted.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::*;
use crate::errors::code;
use crate::git::oauth::store::OAuthTokenStore;
use crate::git::provider::model::{PrSignals, PullRequest};
use crate::mcp::ToolRegistry;
use crate::tools::admin::test_support::Fixture;

const OWNER: &str = "owner@test.com";

const HOSTS: &[(&str, &str)] = &[
    ("github.com", "github"),
    ("github.ibm.com", "github"),
    ("gitlab.com", "gitlab"),
    ("gitlab.example.test", "gitlab"),
    // Declared, and deliberately without a pull request API: the two provider
    // classes FR-NEW-301 refuses before any token is read.
    ("git.acme.internal", "generic"),
    ("public.example.org", "anonymous"),
];

/// mount -> remote url. Four mounts, because every operation needs a GitHub and
/// a GitLab case plus one GitHub Enterprise and one self hosted GitLab; the last
/// two carry the provider classes with no pull request API.
const MOUNTS: &[(&str, &str)] = &[
    ("acme-api", "https://github.com/acme/api.git"),
    ("acme-lab", "https://gitlab.com/acme/api.git"),
    ("acme-ent", "https://github.ibm.com/acme/api.git"),
    ("acme-self", "https://gitlab.example.test/acme/api.git"),
    ("acme-generic", "https://git.acme.internal/acme/api.git"),
    ("acme-pub", "https://public.example.org/acme/api.git"),
];

/// An initialized volume that never recorded an origin row (E2E-NEW-707).
const MOUNT_WITHOUT_REMOTE: &str = "acme-nor";

const GH_TOKEN: &str = "ghp_AAAABBBBCCCCDDDDEEEEFFFF0001";
const GL_TOKEN: &str = "glpat_AAAABBBBCCCCDDDDEEEE0002";

const UNICODE_TITLE: &str = "Ajout des outils PR — été 2026 ✅";
const UNICODE_BODY: &str = "Résumé:\n- prise en charge des MR\n- 日本語テスト";

// ── canned provider payloads ────────────────────────────────────────────────

fn gh_pr_42_open() -> Value {
    json!({
        "number": 42,
        "state": "open",
        "title": "Add PR tools",
        "body": "Adds git.pr_* over the provider REST API.",
        "draft": false,
        "merged": false,
        "merged_at": null,
        "base": {"ref": "main"},
        "head": {"ref": "feature/pr-tools", "sha": "9f1c4a2bdeadbeef0123456789abcdef01234567"},
        "user": {"login": "smorand"},
        "html_url": "https://github.com/acme/api/pull/42",
        "created_at": "2026-09-01T10:00:00Z",
        "updated_at": "2026-09-02T11:30:00Z",
        "commits": 3,
        "changed_files": 4,
        "additions": 120,
        "deletions": 11,
        "mergeable": true,
    })
}

fn gl_mr_42_open() -> Value {
    json!({
        "iid": 42,
        "state": "opened",
        "title": "Add PR tools",
        "description": "Adds git.pr_* over the provider REST API.",
        "draft": false,
        "target_branch": "main",
        "source_branch": "feature/pr-tools",
        "author": {"username": "smorand"},
        "web_url": "https://gitlab.com/acme/api/-/merge_requests/42",
        "created_at": "2026-09-01T10:00:00.000Z",
        "updated_at": "2026-09-02T11:30:00.000Z",
        "commits_count": 3,
        // Deliberately the STRING "4": GitLab reports it that way.
        "changes_count": "4",
        "diff_stats_summary": {"additions": 120, "deletions": 11},
        "merge_status": "can_be_merged",
    })
}

/// The signals E2E-NEW-709's reviews and check-runs routes would supply.
fn approved_and_green() -> PrSignals {
    PrSignals {
        review_state: Some("approved".to_string()),
        checks_state: Some("success".to_string()),
    }
}

// ── the injected fake provider API ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedCall {
    method: String,
    base_url: String,
    path: String,
    query: Vec<(String, String)>,
    body: Option<Value>,
    /// The `Accept` the tool negotiated, so a diff test can prove it asked for
    /// a diff media type rather than JSON.
    accept: Option<String>,
    /// The status the fake answered. `599` is the unrouted sentinel: a test
    /// asserts its absence to prove nothing escaped the canned routes.
    status: u16,
}

/// One canned answer. Carries bytes rather than a `Value` because `git.pr_diff`
/// negotiates `text/plain` and its body is not JSON at all.
#[derive(Clone)]
struct Canned {
    status: u16,
    body: Vec<u8>,
    content_type: Option<String>,
}

impl Canned {
    fn json(status: u16, body: &Value) -> Self {
        Self {
            status,
            body: body.to_string().into_bytes(),
            content_type: Some("application/json".to_string()),
        }
    }
}

/// `(method, path, query) -> (status, body)`, the canned route table. The query
/// part is `None` for a route that answers whatever the query string is, and
/// `Some(..)` for one that answers a single exact query (a later page, or one
/// `state` value among several), which is what lets a list test route two
/// answers to the same path.
type RouteKey = (String, String, Option<String>);
type Routes = HashMap<RouteKey, Canned>;

/// The query string a route key is matched on: `k=v` pairs in request order.
fn query_key(query: &[(String, String)]) -> String {
    query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

#[derive(Clone, Default)]
struct MockProviderApi {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    routes: Arc<Mutex<Routes>>,
}

impl MockProviderApi {
    fn route(&self, method: &str, path: &str, status: u16, body: Value) {
        self.routes
            .lock()
            .unwrap()
            .insert((method.to_string(), path.to_string(), None), Canned::json(status, &body));
    }

    /// A route answering a non JSON body under a stated content type, which is
    /// what a provider's diff endpoint does.
    fn route_text(&self, method: &str, path: &str, status: u16, body: &str, content_type: &str) {
        self.routes.lock().unwrap().insert(
            (method.to_string(), path.to_string(), None),
            Canned {
                status,
                body: body.as_bytes().to_vec(),
                content_type: Some(content_type.to_string()),
            },
        );
    }

    /// A route that answers only one exact query string.
    fn route_q(&self, method: &str, path: &str, query: &str, status: u16, body: Value) {
        self.routes.lock().unwrap().insert(
            (method.to_string(), path.to_string(), Some(query.to_string())),
            Canned::json(status, &body),
        );
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }

    fn steps(&self) -> Vec<(String, String)> {
        self.calls().into_iter().map(|c| (c.method, c.path)).collect()
    }

    fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }
}

#[async_trait]
impl ProviderClient for MockProviderApi {
    async fn send(&self, req: &ProviderRequest, cred: &Credential) -> Result<ProviderResponse> {
        // A credential must reach the transport and nothing else: proving it is
        // present here is what makes the "never in a response" assertions mean
        // something.
        assert!(!cred.header_value().is_empty(), "the transport receives the credential");
        let method = req.method.as_str().to_string();
        let canned = {
            let routes = self.routes.lock().unwrap();
            // Exact query first, then the query agnostic route.
            routes
                .get(&(method.clone(), req.path.clone(), Some(query_key(&req.query))))
                .or_else(|| routes.get(&(method.clone(), req.path.clone(), None)))
                .cloned()
                .unwrap_or_else(|| Canned::json(599, &json!({"message": "unrouted"})))
        };
        self.calls.lock().unwrap().push(RecordedCall {
            method,
            base_url: req.base_url.clone(),
            path: req.path.clone(),
            query: req.query.clone(),
            body: req.body.clone(),
            accept: req.accept.clone(),
            status: canned.status,
        });
        // The fake enforces the caller's cap exactly as the real transport
        // does, so a truncation test exercises the tool's own accounting.
        let mut body = canned.body;
        let mut truncated = false;
        if let Some(cap) = req.truncate_at
            && body.len() > cap
        {
            body.truncate(cap);
            truncated = true;
        }
        Ok(ProviderResponse {
            status: canned.status,
            body,
            content_type: canned.content_type,
            truncated,
        })
    }
}

// ── environment ─────────────────────────────────────────────────────────────

struct PrEnv {
    f: Fixture,
    reg: ToolRegistry,
    api: Arc<MockProviderApi>,
    tokens: Arc<OAuthTokenStore>,
}

impl PrEnv {
    /// Four initialized mounts, each with its `origin` row, and a token for
    /// every host with the scopes the write surface needs.
    async fn new() -> PrEnv {
        Self::build(&[
            ("github.com", "github", GH_TOKEN, &["repo"]),
            ("github.ibm.com", "github", GH_TOKEN, &["repo"]),
            ("gitlab.com", "gitlab", GL_TOKEN, &["api"]),
            ("gitlab.example.test", "gitlab", GL_TOKEN, &["api"]),
        ])
        .await
    }

    async fn build(tokens_for: &[(&str, &str, &str, &[&str])]) -> PrEnv {
        Self::build_with(tokens_for, None).await
    }

    /// `diff_cap_mb` overrides `git.max_pr_diff_mb`, so a truncation test does
    /// not have to build a body the size of the 12 MiB default.
    async fn build_with(
        tokens_for: &[(&str, &str, &str, &[&str])],
        diff_cap_mb: Option<usize>,
    ) -> PrEnv {
        let owned: Vec<(String, String)> =
            HOSTS.iter().map(|(h, p)| ((*h).to_string(), (*p).to_string())).collect();
        let f = Fixture::with_config(move |c| {
            c.git.enabled = true;
            c.git.hosts.0 = owned;
            if let Some(mb) = diff_cap_mb {
                c.git.max_pr_diff_mb = mb;
            }
            crate::git::remote::validate_hosts(&c.git).expect("a valid host map");
        })
        .await;
        let git =
            Arc::new(GitRepoStore::new(f.state.config.clone(), crate::storage::test_registry()));
        for (mount, url) in MOUNTS {
            f.seed_project(mount, OWNER).await;
            git.init_repo(mount).await.unwrap();
            git.get_db(mount).await.unwrap().add_remote("origin", url).await.unwrap();
        }
        // Initialized, no origin row: the FR-NEW-302 offline refusal.
        f.seed_project(MOUNT_WITHOUT_REMOTE, OWNER).await;
        git.init_repo(MOUNT_WITHOUT_REMOTE).await.unwrap();
        let tokens = Arc::new(OAuthTokenStore::new());
        for (host, provider, token, scopes) in tokens_for {
            // A self hosted GitLab records the API base alongside the token,
            // which is what `acme-self` proves flows into base resolution.
            let instance = (*host == "gitlab.example.test").then(|| format!("https://{host}"));
            tokens
                .store_token(
                    OWNER,
                    host,
                    provider,
                    token,
                    scopes.iter().map(|s| (*s).to_string()).collect(),
                    Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                    instance,
                )
                .await
                .unwrap();
        }
        let api = Arc::new(MockProviderApi::default());
        let mut reg = ToolRegistry::new();
        register_with(
            &mut reg,
            Some(git),
            Some(tokens.clone()),
            Some(api.clone() as Arc<dyn ProviderClient>),
        );
        PrEnv { f, reg, api, tokens }
    }

    async fn create(&self, args: Value) -> Result<Value> {
        self.f.call(&self.reg, OWNER, "git.pr_create", args).await
    }

    async fn list(&self, args: Value) -> Result<Value> {
        self.f.call(&self.reg, OWNER, "git.pr_list", args).await
    }

    async fn get(&self, args: Value) -> Result<Value> {
        self.f.call(&self.reg, OWNER, "git.pr_get", args).await
    }

    async fn diff(&self, args: Value) -> Result<Value> {
        self.f.call(&self.reg, OWNER, "git.pr_diff", args).await
    }

    async fn merge(&self, args: Value) -> Result<Value> {
        self.f.call(&self.reg, OWNER, "git.pr_merge", args).await
    }

    async fn review(&self, args: Value) -> Result<Value> {
        self.f.call(&self.reg, OWNER, "git.pr_review", args).await
    }
}

/// Serialize against every other test touching the process wide `git.hosts`
/// map, then run the body on a fresh single test runtime.
fn with_hosts_lock<F: std::future::Future>(f: F) -> F::Output {
    let _guard = crate::git::remote::tests::lock_for_test();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
}

/// Every key of a JSON document, recursively, so a leaked `authorization` or
/// `headers` cannot hide in a nested object.
fn all_keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                out.push(k.clone());
                all_keys(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| all_keys(i, out)),
        _ => {}
    }
}

const NORMALIZED_KEYS: [&str; 21] = [
    "provider",
    "host",
    "number",
    "title",
    "body",
    "state",
    "draft",
    "base",
    "head",
    "author",
    "url",
    "created_at",
    "updated_at",
    "commits",
    "changed_files",
    "additions",
    "deletions",
    "review_state",
    "checks_state",
    "mergeable",
    "raw",
];

fn keys_of(v: &Value) -> Vec<String> {
    v.as_object().expect("an object").keys().cloned().collect()
}

// ── schema and registration ─────────────────────────────────────────────────

#[test]
fn git_pr_create_schema_matches_the_contract() {
    let mut r = ToolRegistry::new();
    register(&mut r);
    assert_eq!(r.len(), 6);
    for name in ["git.pr_list", "git.pr_get", "git.pr_diff", "git.pr_merge", "git.pr_review"] {
        assert!(r.resolve(name).is_some(), "{name} must be registered");
    }
    let s = &r.resolve("git.pr_create").unwrap().schema;
    let expected: Value = serde_json::from_str(
        r#"{"type":"object","properties":{
             "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
             "base":{"description":"Branch the pull request merges into, on the remote.","type":"string"},
             "head":{"description":"Branch holding the changes; it must already exist on the remote.","type":"string"},
             "title":{"description":"Title of the pull request.","type":"string"},
             "body":{"description":"Description of the pull request.","type":"string","default":null},
             "draft":{"description":"Open the pull request as a draft; on GitLab, which has no draft flag, the title is prefixed with 'Draft: '.","type":"boolean","default":false},
             "remote":{"description":"Name of the declared remote to open it on; defaults to origin.","type":"string","default":"origin"}},
           "required":["mount_id","base","head","title"]}"#,
    )
    .unwrap();
    assert_eq!(s.input_schema(), expected);
}

// ── FR-NEW-303 / FR-NEW-317: the normalized model ───────────────────────────

/// E2E-NEW-709, the normalization proof. `git.pr_get` is a later story, so the
/// two canned payloads go through the mapper directly, with the signals its
/// reviews and check-runs routes would have supplied.
#[test]
fn e2e_new_709_both_providers_normalize_to_one_identical_object() {
    let signals = approved_and_green();
    let gh = PullRequest::from_github("github.com", gh_pr_42_open(), &signals).to_value();
    let gl = PullRequest::from_gitlab("gitlab.com", gl_mr_42_open(), &signals).to_value();

    assert_eq!(keys_of(&gh), NORMALIZED_KEYS, "the frozen key set, in order");
    assert_eq!(keys_of(&gh), keys_of(&gl), "one key set for both providers, order included");

    for k in NORMALIZED_KEYS {
        if ["provider", "host", "url", "raw"].contains(&k) {
            continue;
        }
        assert_eq!(gh[k], gl[k], "key '{k}' differs between providers");
    }
    assert_eq!(gh["number"], 42);
    assert_eq!(gh["title"], "Add PR tools");
    assert_eq!(gh["body"], "Adds git.pr_* over the provider REST API.");
    assert_eq!(gh["state"], "open");
    assert_eq!(gh["draft"], false);
    assert_eq!(gh["base"], "main");
    assert_eq!(gh["head"], "feature/pr-tools");
    assert_eq!(gh["author"], "smorand");
    assert_eq!(gh["created_at"], "2026-09-01T10:00:00+00:00");
    assert_eq!(gh["updated_at"], "2026-09-02T11:30:00+00:00");
    assert_eq!(gh["commits"], 3);
    assert_eq!(gh["changed_files"], 4, "GitLab's string \"4\" normalizes to the number 4");
    assert_eq!(gl["changed_files"], 4);
    assert_eq!(gh["additions"], 120);
    assert_eq!(gh["deletions"], 11);
    assert_eq!(gh["review_state"], "approved");
    assert_eq!(gh["checks_state"], "success");
    assert_eq!(gh["mergeable"], true);

    assert_eq!(gh["provider"], "github");
    assert_eq!(gl["provider"], "gitlab");
    assert_eq!(gh["url"], "https://github.com/acme/api/pull/42");
    assert_eq!(gl["url"], "https://gitlab.com/acme/api/-/merge_requests/42");

    // raw stays provider shaped, never normalized.
    assert_eq!(gh["raw"]["head"]["sha"], "9f1c4a2bdeadbeef0123456789abcdef01234567");
    assert_eq!(gl["raw"]["iid"], 42);
}

/// E2E-NEW-735: a merged pull request on both providers, and GitHub's
/// uncomputed mergeability stays JSON null rather than becoming false.
#[test]
fn e2e_new_735_merged_normalizes_on_both_and_mergeable_stays_null() {
    let mut gh_raw = gh_pr_42_open();
    gh_raw["state"] = json!("closed");
    gh_raw["merged"] = json!(true);
    gh_raw["merged_at"] = json!("2026-09-03T08:00:00Z");
    gh_raw["mergeable"] = Value::Null;
    let mut gl_raw = gl_mr_42_open();
    gl_raw["state"] = json!("merged");
    gl_raw["merge_status"] = json!("cannot_be_merged");

    let gh = PullRequest::from_github("github.com", gh_raw, &PrSignals::unknown()).to_value();
    let gl = PullRequest::from_gitlab("gitlab.com", gl_raw, &PrSignals::unknown()).to_value();
    assert_eq!(gh["state"], "merged");
    assert_eq!(gl["state"], "merged");
    assert_eq!(gh["mergeable"], Value::Null, "null, not false");
    assert!(gh["mergeable"].is_null());
    assert_eq!(gl["mergeable"], false);
}

/// E2E-NEW-736: GitHub's closed and merged look alike without `merged_at`.
#[test]
fn e2e_new_736_closed_unmerged_is_closed_on_both() {
    let mut gh_raw = gh_pr_42_open();
    gh_raw["state"] = json!("closed");
    gh_raw["merged"] = json!(false);
    gh_raw["merged_at"] = Value::Null;
    let mut gl_raw = gl_mr_42_open();
    gl_raw["state"] = json!("closed");

    let gh = PullRequest::from_github("github.com", gh_raw, &PrSignals::unknown()).to_value();
    let gl = PullRequest::from_gitlab("gitlab.com", gl_raw, &PrSignals::unknown()).to_value();
    assert_eq!(gh["state"], "closed");
    assert_eq!(gl["state"], "closed");
    assert_ne!(gh["state"], "merged");
    assert_ne!(gl["state"], "merged");
}

/// E2E-NEW-737: draft is a separate boolean, never folded into state, and
/// GitLab's `Draft: ` title prefix is stripped so both titles agree.
#[test]
fn e2e_new_737_draft_is_separate_and_the_gitlab_prefix_is_stripped() {
    let mut gh_raw = gh_pr_42_open();
    gh_raw["draft"] = json!(true);
    gh_raw["state"] = json!("open");
    let mut gl_raw = gl_mr_42_open();
    gl_raw["draft"] = json!(true);
    gl_raw["state"] = json!("opened");
    gl_raw["title"] = json!("Draft: Add PR tools");

    let gh = PullRequest::from_github("github.com", gh_raw, &PrSignals::unknown()).to_value();
    let gl = PullRequest::from_gitlab("gitlab.com", gl_raw, &PrSignals::unknown()).to_value();
    for v in [&gh, &gl] {
        assert_eq!(v["state"], "open");
        assert_eq!(v["draft"], true);
        assert_eq!(v["title"], "Add PR tools");
    }
}

/// E2E-NEW-798: unicode survives normalization byte for byte, on a get shaped
/// payload and on the list shaped one (the same payload minus the counts, which
/// is what `git.pr_list` will hand the mapper in the next story).
#[test]
fn e2e_new_798_unicode_titles_and_bodies_round_trip_unchanged() {
    assert_eq!(UNICODE_TITLE.chars().count(), 32, "a mojibake round trip must fail loudly");

    let mut gh_raw = gh_pr_42_open();
    gh_raw["title"] = json!(UNICODE_TITLE);
    gh_raw["body"] = json!(UNICODE_BODY);
    let mut gl_raw = gl_mr_42_open();
    gl_raw["title"] = json!(UNICODE_TITLE);
    gl_raw["description"] = json!(UNICODE_BODY);

    let mut gh_list = gh_raw.clone();
    for k in ["commits", "changed_files", "additions", "deletions", "mergeable"] {
        gh_list.as_object_mut().unwrap().remove(k);
    }
    let mut gl_list = gl_raw.clone();
    for k in ["commits_count", "changes_count", "diff_stats_summary", "merge_status"] {
        gl_list.as_object_mut().unwrap().remove(k);
    }

    for v in [
        PullRequest::from_github("github.com", gh_raw, &PrSignals::unknown()).to_value(),
        PullRequest::from_gitlab("gitlab.com", gl_raw, &PrSignals::unknown()).to_value(),
        PullRequest::from_github("github.com", gh_list, &PrSignals::unknown()).to_value(),
        PullRequest::from_gitlab("gitlab.com", gl_list, &PrSignals::unknown()).to_value(),
    ] {
        assert_eq!(v["title"], UNICODE_TITLE);
        assert_eq!(v["body"], UNICODE_BODY);
        assert_eq!(v["title"].as_str().unwrap().chars().count(), 32);
    }
}

// ── FR-NEW-304: create, on both providers and both deployments ──────────────

/// E2E-NEW-710: GitHub, the head branch pre-flight then the create.
#[test]
fn e2e_new_710_pr_create_on_github() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "GET",
            "/repos/acme/api/branches/feature/pr-tools",
            200,
            json!({"name": "feature/pr-tools", "commit": {"sha": "9f1c4a2b"}}),
        );
        e.api.route("POST", "/repos/acme/api/pulls", 201, gh_pr_42_open());

        let out = e
            .create(json!({
                "mount_id": "acme-api", "base": "main", "head": "feature/pr-tools",
                "title": "Add PR tools", "body": "Adds git.pr_* over the provider REST API.",
            }))
            .await
            .unwrap();

        assert_eq!(
            e.api.steps(),
            vec![
                ("GET".to_string(), "/repos/acme/api/branches/feature/pr-tools".to_string()),
                ("POST".to_string(), "/repos/acme/api/pulls".to_string()),
            ]
        );
        let post = &e.api.calls()[1];
        assert_eq!(post.base_url, "https://api.github.com");
        assert_eq!(
            post.body.clone().unwrap(),
            json!({
                "base": "main", "head": "feature/pr-tools", "title": "Add PR tools",
                "body": "Adds git.pr_* over the provider REST API.", "draft": false,
            })
        );
        assert_eq!(out["number"], 42);
        assert_eq!(out["state"], "open");
        assert_eq!(out["draft"], false);
        assert_eq!(out["provider"], "github");
        assert_eq!(out["host"], "github.com");
    });
}

/// E2E-NEW-711: GitLab creates a merge request, with GitLab's own field names,
/// and normalizes to the same object.
#[test]
fn e2e_new_711_pr_create_on_gitlab() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/repository/branches/feature/pr-tools",
            200,
            json!({"name": "feature/pr-tools"}),
        );
        e.api.route("POST", "/projects/acme%2Fapi/merge_requests", 201, gl_mr_42_open());

        let out = e
            .create(json!({
                "mount_id": "acme-lab", "base": "main", "head": "feature/pr-tools",
                "title": "Add PR tools", "body": "Adds git.pr_* over the provider REST API.",
            }))
            .await
            .unwrap();

        let post = &e.api.calls()[1];
        assert_eq!(
            post.body.clone().unwrap(),
            json!({
                "source_branch": "feature/pr-tools", "target_branch": "main",
                "title": "Add PR tools",
                "description": "Adds git.pr_* over the provider REST API.",
            })
        );
        assert_eq!(out["number"], 42);
        assert_eq!(out["provider"], "gitlab");
        assert_eq!(keys_of(&out), NORMALIZED_KEYS);
    });
}

/// E2E-NEW-712: GitHub Enterprise Server, every call on `/api/v3`.
#[test]
fn e2e_new_712_pr_create_on_github_enterprise_resolves_the_instance_base() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/branches/feature/pr-tools", 200, json!({"name": "x"}));
        e.api.route("POST", "/repos/acme/api/pulls", 201, gh_pr_42_open());

        let out = e
            .create(json!({
                "mount_id": "acme-ent", "base": "main", "head": "feature/pr-tools",
                "title": "Add PR tools",
            }))
            .await
            .unwrap();

        assert_eq!(e.api.calls().len(), 2);
        for c in e.api.calls() {
            assert_eq!(c.base_url, "https://github.ibm.com/api/v3");
        }
        assert_eq!(out["host"], "github.ibm.com");
    });
}

/// E2E-NEW-713: a self hosted GitLab, whose API base comes from the instance URL
/// recorded with the token, and unicode passed through byte for byte.
#[test]
fn e2e_new_713_pr_create_on_self_hosted_gitlab_keeps_unicode_byte_for_byte() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/repository/branches/feature/pr-tools",
            200,
            json!({"name": "feature/pr-tools"}),
        );
        let mut created = gl_mr_42_open();
        created["title"] = json!(UNICODE_TITLE);
        created["description"] = json!(UNICODE_BODY);
        e.api.route("POST", "/projects/acme%2Fapi/merge_requests", 201, created);

        let out = e
            .create(json!({
                "mount_id": "acme-self", "base": "main", "head": "feature/pr-tools",
                "title": UNICODE_TITLE, "body": UNICODE_BODY,
            }))
            .await
            .unwrap();

        let post = &e.api.calls()[1];
        assert_eq!(post.base_url, "https://gitlab.example.test/api/v4");
        let body = post.body.clone().unwrap();
        assert_eq!(body["title"].as_str().unwrap(), UNICODE_TITLE);
        assert_eq!(body["description"].as_str().unwrap(), UNICODE_BODY);
        assert_eq!(out["title"], UNICODE_TITLE);
        assert_eq!(out["body"], UNICODE_BODY);
        assert_eq!(out["host"], "gitlab.example.test");
    });
}

/// E2E-NEW-925: the success path plus the ordering guarantee, then the same
/// fixture with the pre-flight failing, so the ordering is visible from both
/// sides in one test.
#[test]
fn e2e_new_925_the_head_pre_flight_precedes_the_create_and_gates_it() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "GET",
            "/repos/acme/api/branches/feature/login",
            200,
            json!({"name": "feature/login", "commit": {"sha": "abc123deadbeef"}}),
        );
        let mut created = gh_pr_42_open();
        created["number"] = json!(7);
        created["base"] = json!({"ref": "main"});
        created["head"] = json!({"ref": "feature/login"});
        e.api.route("POST", "/repos/acme/api/pulls", 201, created);

        let args = json!({
            "mount_id": "acme-api", "base": "main", "head": "feature/login",
            "title": "Add login", "body": "why",
        });
        let out = e.create(args.clone()).await.unwrap();
        assert_eq!(out["number"], 7);
        assert_eq!(out["state"], "open");
        assert_eq!(out["base"], "main");
        assert_eq!(out["head"], "feature/login");
        assert_eq!(
            e.api.steps(),
            vec![
                ("GET".to_string(), "/repos/acme/api/branches/feature/login".to_string()),
                ("POST".to_string(), "/repos/acme/api/pulls".to_string()),
            ]
        );
        assert_eq!(
            e.api.calls()[1].body.clone().unwrap(),
            json!({"base": "main", "head": "feature/login", "title": "Add login",
                   "body": "why", "draft": false})
        );

        // Same call, pre-flight now 404: one call only, and it is the GET.
        e.api.clear();
        e.api.route(
            "GET",
            "/repos/acme/api/branches/feature/login",
            404,
            json!({"message": "Branch not found"}),
        );
        let err = e.create(args).await.unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
        for part in ["feature/login", "git.remote_push"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert_eq!(e.api.steps().len(), 1);
        assert_eq!(e.api.steps()[0].0, "GET");
    });
}

/// E2E-NEW-721: draft on both providers. GitHub carries a flag, GitLab a title
/// prefix, and the normalized object agrees.
#[test]
fn e2e_new_721_draft_on_github_is_a_flag_and_on_gitlab_a_title_prefix() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/branches/feature/pr-tools", 200, json!({"name": "x"}));
        let mut gh_created = gh_pr_42_open();
        gh_created["draft"] = json!(true);
        e.api.route("POST", "/repos/acme/api/pulls", 201, gh_created);

        let gh = e
            .create(json!({
                "mount_id": "acme-api", "base": "main", "head": "feature/pr-tools",
                "title": "Add PR tools", "draft": true,
            }))
            .await
            .unwrap();
        assert_eq!(e.api.calls()[1].body.clone().unwrap()["draft"], true);
        assert_eq!(gh["draft"], true);
        assert_eq!(gh["state"], "open");

        e.api.clear();
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/repository/branches/feature/pr-tools",
            200,
            json!({"name": "x"}),
        );
        let mut gl_created = gl_mr_42_open();
        gl_created["title"] = json!("Draft: Add PR tools");
        gl_created["draft"] = json!(true);
        e.api.route("POST", "/projects/acme%2Fapi/merge_requests", 201, gl_created);

        let gl = e
            .create(json!({
                "mount_id": "acme-lab", "base": "main", "head": "feature/pr-tools",
                "title": "Add PR tools", "draft": true,
            }))
            .await
            .unwrap();
        let body = e.api.calls()[1].body.clone().unwrap();
        assert_eq!(body["title"], "Draft: Add PR tools", "GitLab has no draft flag");
        assert!(body.get("draft").is_none());
        assert_eq!(gl["draft"], true);
        assert_eq!(gl["title"], "Add PR tools", "the prefix is stripped so both agree");
    });
}

// ── FR-NEW-305 and argument validation: rejected before any network call ─────

/// E2E-NEW-714: GitHub, head branch absent on the remote.
#[test]
fn e2e_new_714_create_fails_early_when_head_is_not_on_the_github_remote() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "GET",
            "/repos/acme/api/branches/feature/ghost",
            404,
            json!({"message": "Branch not found"}),
        );

        let err = e
            .create(json!({
                "mount_id": "acme-api", "base": "main", "head": "feature/ghost", "title": "x",
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
        assert!(
            err.message.contains("head branch 'feature/ghost' does not exist on host 'github.com'"),
            "got: {}",
            err.message
        );
        assert!(err.message.contains("git.remote_push"));
        assert_eq!(e.api.calls().len(), 1, "no POST was issued");
        assert_eq!(
            e.api.steps()[0],
            ("GET".to_string(), "/repos/acme/api/branches/feature/ghost".to_string())
        );
    });
}

/// E2E-NEW-715: the same on GitLab, whose 404 body differs.
#[test]
fn e2e_new_715_create_fails_early_when_head_is_not_on_the_gitlab_remote() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/repository/branches/feature/ghost",
            404,
            json!({"message": "404 Branch Not Found"}),
        );

        let err = e
            .create(json!({
                "mount_id": "acme-lab", "base": "main", "head": "feature/ghost", "title": "x",
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
        assert!(
            err.message.contains("head branch 'feature/ghost' does not exist on host 'gitlab.com'"),
            "got: {}",
            err.message
        );
        assert_eq!(e.api.calls().len(), 1);
        assert_eq!(e.api.steps()[0].0, "GET");
    });
}

/// E2E-NEW-716: base and head naming the same branch is nonsense, and costs no
/// network call to say so.
#[test]
fn e2e_new_716_base_and_head_must_differ() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let err = e
            .create(json!({
                "mount_id": "acme-api", "base": "main", "head": "main", "title": "x",
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("base and head must differ, both are 'main'"),
            "got: {}",
            err.message
        );
        assert!(e.api.calls().is_empty());
    });
}

/// E2E-NEW-717: a whitespace only title.
#[test]
fn e2e_new_717_title_must_not_be_whitespace_only() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let err = e
            .create(json!({
                "mount_id": "acme-api", "base": "main", "head": "feature/x", "title": "   ",
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("title must not be empty or whitespace-only"),
            "got: {}",
            err.message
        );
        assert!(e.api.calls().is_empty());
    });
}

/// E2E-NEW-777: the scope gate precedes every network call, the head branch
/// pre-flight included.
#[test]
fn e2e_new_777_an_insufficient_scope_set_is_refused_before_the_pre_flight() {
    with_hosts_lock(async {
        let e = PrEnv::build(&[("gitlab.com", "gitlab", GL_TOKEN, &["read_api"])]).await;
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/repository/branches/feature/pr-tools",
            200,
            json!({"name": "x"}),
        );
        e.api.route("POST", "/projects/acme%2Fapi/merge_requests", 201, gl_mr_42_open());

        let err = e
            .create(json!({
                "mount_id": "acme-lab", "base": "main", "head": "feature/pr-tools", "title": "x",
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::FORBIDDEN);
        for part in ["missing scope 'api'", "git.pr_create"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(e.api.calls().is_empty(), "not even the pre-flight ran");
    });
}

/// A provider side rejection is surfaced faithfully, never swallowed and never
/// reported as success (FR-NEW-304): a duplicate pull request keeps GitHub's
/// status and message.
#[test]
fn a_provider_rejection_is_surfaced_with_its_status_and_message() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/branches/feature/pr-tools", 200, json!({"name": "x"}));
        e.api.route(
            "POST",
            "/repos/acme/api/pulls",
            422,
            json!({"message": "A pull request already exists for acme:feature/pr-tools."}),
        );

        let err = e
            .create(json!({
                "mount_id": "acme-api", "base": "main", "head": "feature/pr-tools",
                "title": "Add PR tools",
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        for part in ["422", "A pull request already exists", "github.com"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert_eq!(e.api.calls().len(), 2, "the create was attempted and refused");
    });
}

// ── membership and security ─────────────────────────────────────────────────

/// Membership only: a platform admin who is not a member gets ERR_FORBIDDEN,
/// and nothing is sent.
#[test]
fn a_platform_admin_who_is_not_a_member_cannot_create_a_pull_request() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let err =
            e.f.call(
                &e.reg,
                crate::tools::admin::test_support::ADMIN,
                "git.pr_create",
                json!({"mount_id": "acme-api", "base": "main", "head": "f", "title": "t"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::FORBIDDEN);
        assert!(e.api.calls().is_empty());
    });
}

/// E2E-NEW-793: `raw` is exactly the canned payload and nothing more, and no
/// token value nor request metadata reaches the response. Asserted on
/// `git.pr_create`, the tool this story ships.
#[test]
fn e2e_new_793_raw_is_the_untouched_payload_and_carries_no_credential() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/branches/feature/pr-tools", 200, json!({"name": "x"}));
        e.api.route("POST", "/repos/acme/api/pulls", 201, gh_pr_42_open());
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/repository/branches/feature/pr-tools",
            200,
            json!({"name": "x"}),
        );
        e.api.route("POST", "/projects/acme%2Fapi/merge_requests", 201, gl_mr_42_open());

        for (mount, canned) in [("acme-api", gh_pr_42_open()), ("acme-lab", gl_mr_42_open())] {
            let out = e
                .create(json!({
                    "mount_id": mount, "base": "main", "head": "feature/pr-tools",
                    "title": "Add PR tools",
                }))
                .await
                .unwrap();
            assert_eq!(out["raw"], canned, "raw deep-equals the canned payload");

            let mut keys = Vec::new();
            all_keys(&out, &mut keys);
            for forbidden in ["authorization", "token", "headers", "request"] {
                assert!(
                    !keys.iter().any(|k| k.eq_ignore_ascii_case(forbidden)),
                    "key '{forbidden}' is in the response of {mount}"
                );
            }
            let serialized = out.to_string();
            for secret in ["ghp_", "glpat_"] {
                assert!(
                    !serialized.contains(secret),
                    "'{secret}' leaked into the {mount} response"
                );
            }
        }
        // And the stored tokens are still exactly what was seeded: nothing here
        // rewrote or logged them.
        assert_eq!(e.tokens.get_token(OWNER, "github.com").unwrap().access_token, GH_TOKEN);
    });
}

/// E2E-NEW-926: one injected client serves a GitHub and a GitLab mount in the
/// same process. `git.pr_get` is a later story, so this drives the seam through
/// `git.pr_create` on both providers.
#[test]
fn e2e_new_926_one_injected_client_serves_both_providers() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/branches/feature/pr-tools", 200, json!({"name": "x"}));
        e.api.route("POST", "/repos/acme/api/pulls", 201, gh_pr_42_open());
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/repository/branches/feature/pr-tools",
            200,
            json!({"name": "x"}),
        );
        e.api.route("POST", "/projects/acme%2Fapi/merge_requests", 201, gl_mr_42_open());

        let args = |mount: &str| {
            json!({"mount_id": mount, "base": "main", "head": "feature/pr-tools",
                   "title": "Add PR tools"})
        };
        let gh = e.create(args("acme-api")).await.unwrap();
        let gl = e.create(args("acme-lab")).await.unwrap();

        assert_eq!(keys_of(&gh), keys_of(&gl), "identical key set, identical order");
        for k in NORMALIZED_KEYS {
            if ["provider", "host", "url", "raw"].contains(&k) {
                continue;
            }
            assert_eq!(gh[k], gl[k], "key '{k}' differs");
        }
        assert_eq!(gh["provider"], "github");
        assert_eq!(gl["provider"], "gitlab");
        assert_ne!(gh["host"], gl["host"]);

        let bases: Vec<String> = e.api.calls().into_iter().map(|c| c.base_url).collect();
        assert!(bases.iter().any(|b| b == "https://api.github.com"));
        assert!(bases.iter().any(|b| b == "https://gitlab.com/api/v4"));
        assert!(
            e.api.calls().iter().all(|c| c.status != 599),
            "nothing was unrouted, so no real network client was built"
        );
    });
}

/// No token value reaches a tracing span or an event, on the success path or on
/// the provider rejection path. The property US-024 proved for the transport,
/// extended to this tool.
#[test]
fn no_token_value_reaches_a_tracing_span_or_a_log_line() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/branches/feature/pr-tools", 200, json!({"name": "x"}));
        e.api.route("POST", "/repos/acme/api/pulls", 201, gh_pr_42_open());

        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || CaptureWriter(writer.clone()))
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);

        let ok = e
            .create(json!({"mount_id": "acme-api", "base": "main", "head": "feature/pr-tools",
                           "title": "Add PR tools"}))
            .await
            .unwrap();
        e.api.route("POST", "/repos/acme/api/pulls", 403, json!({"message": "Resource protected"}));
        let err = e
            .create(json!({"mount_id": "acme-api", "base": "main", "head": "feature/pr-tools",
                           "title": "Add PR tools"}))
            .await
            .unwrap_err();
        drop(guard);

        let logged = String::from_utf8_lossy(&buf.lock().unwrap().clone()).to_string();
        for text in [logged.as_str(), &ok.to_string(), &err.message] {
            for secret in [GH_TOKEN, GL_TOKEN, "ghp_", "glpat_"] {
                assert!(!text.contains(secret), "'{secret}' leaked into: {text}");
            }
        }
    });
}

// ── FR-NEW-306: git.pr_list ─────────────────────────────────────────────────

/// The 16 keys a list item carries: the normalized set minus the five that only
/// a single pull request read reports. `raw` IS present.
const LIST_KEYS: [&str; 16] = [
    "provider",
    "host",
    "number",
    "title",
    "body",
    "state",
    "draft",
    "base",
    "head",
    "author",
    "url",
    "created_at",
    "updated_at",
    "review_state",
    "checks_state",
    "raw",
];

const OMITTED_ON_A_LIST_ITEM: [&str; 5] =
    ["commits", "changed_files", "additions", "deletions", "mergeable"];

fn query_of(c: &RecordedCall) -> Vec<(&str, &str)> {
    c.query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// A GitHub list payload entry: the open pull request 42 with the counts a list
/// response does not carry.
fn gh_list_item(number: u64) -> Value {
    let mut v = gh_pr_42_open();
    for k in OMITTED_ON_A_LIST_ITEM {
        v.as_object_mut().unwrap().remove(k);
    }
    v["number"] = json!(number);
    v
}

/// E2E-NEW-700: GitHub, an empty list, and the exact route and query.
#[test]
fn e2e_new_700_pr_list_on_github_calls_the_pulls_route_once() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/pulls", 200, json!([]));

        let out = e.list(json!({"mount_id": "acme-api", "state": "open"})).await.unwrap();
        assert_eq!(out, json!({"pull_requests": [], "count": 0}));

        let calls = e.api.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "GET");
        assert_eq!(calls[0].base_url, "https://api.github.com");
        assert_eq!(calls[0].path, "/repos/acme/api/pulls");
        assert_eq!(query_of(&calls[0]), vec![("state", "open"), ("per_page", "100")]);
        assert!(calls[0].body.is_none());
    });
}

/// E2E-NEW-701: the same route under a GitHub Enterprise `/api/v3` base.
#[test]
fn e2e_new_701_pr_list_on_github_enterprise_uses_the_api_v3_base() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/pulls", 200, json!([]));

        let out = e.list(json!({"mount_id": "acme-ent", "state": "open"})).await.unwrap();
        let calls = e.api.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].base_url, "https://github.ibm.com/api/v3");
        assert_eq!(calls[0].path, "/repos/acme/api/pulls");
        assert_eq!(out["count"], 0);
    });
}

/// E2E-NEW-702: GitLab lists merge requests, and `open` maps to `opened`.
#[test]
fn e2e_new_702_pr_list_on_gitlab_calls_the_merge_requests_route() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/projects/acme%2Fapi/merge_requests", 200, json!([]));

        let out = e.list(json!({"mount_id": "acme-lab", "state": "open"})).await.unwrap();
        let calls = e.api.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "GET");
        assert_eq!(calls[0].base_url, "https://gitlab.com/api/v4");
        assert_eq!(calls[0].path, "/projects/acme%2Fapi/merge_requests");
        assert_eq!(query_of(&calls[0]), vec![("state", "opened"), ("per_page", "100")]);
        assert_eq!(out["count"], 0);
    });
}

/// E2E-NEW-703: a self hosted GitLab resolves its API base from the instance
/// URL recorded with the token.
#[test]
fn e2e_new_703_pr_list_on_self_hosted_gitlab_uses_the_instance_base() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/projects/acme%2Fapi/merge_requests", 200, json!([]));

        e.list(json!({"mount_id": "acme-self", "state": "open"})).await.unwrap();
        let calls = e.api.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].base_url, "https://gitlab.example.test/api/v4");
        assert_eq!(calls[0].path, "/projects/acme%2Fapi/merge_requests");
    });
}

/// E2E-NEW-704: a `generic` host has no pull request API, refused offline.
///
/// The message is the one US-024 froze in `resolve_target`, which names the host
/// and both supported providers; this story does not reword it.
#[test]
fn e2e_new_704_a_generic_host_is_refused_before_any_call() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let err = e.list(json!({"mount_id": "acme-generic", "state": "open"})).await.unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);
        for part in ["git.acme.internal", "github", "gitlab", "git.hosts"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(e.api.calls().is_empty());
    });
}

/// E2E-NEW-705: the same for an `anonymous` host.
#[test]
fn e2e_new_705_an_anonymous_host_is_refused_before_any_call() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let err = e.list(json!({"mount_id": "acme-pub", "state": "open"})).await.unwrap_err();
        assert_eq!(err.code, code::NOT_SUPPORTED);
        for part in ["public.example.org", "github", "gitlab", "git.hosts"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(e.api.calls().is_empty());
    });
}

/// E2E-NEW-707: a volume with no origin row cannot name a provider at all.
#[test]
fn e2e_new_707_a_volume_without_an_origin_remote_is_refused_offline() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let err = e.list(json!({"mount_id": "acme-nor", "state": "open"})).await.unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("volume 'acme-nor' has no origin remote"),
            "got: {}",
            err.message
        );
        assert!(e.api.calls().is_empty());
    });
}

/// E2E-NEW-723: two items, each carrying exactly the 16 list keys in order.
#[test]
fn e2e_new_723_a_list_item_has_exactly_the_sixteen_list_keys() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let mut second = gh_list_item(43);
        second["title"] = json!("Bump deps");
        second["head"] =
            json!({"ref": "chore/bump", "sha": "aa1100000000000000000000000000000000cafe"});
        e.api.route("GET", "/repos/acme/api/pulls", 200, json!([gh_list_item(42), second]));

        let out = e.list(json!({"mount_id": "acme-api", "state": "open"})).await.unwrap();
        assert_eq!(out["count"], 2);
        let items = out["pull_requests"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(keys_of(&items[0]), LIST_KEYS, "the frozen list key set, in order");
        assert_eq!(keys_of(&items[1]), LIST_KEYS);
        assert_eq!(items[0]["number"], 42);
        assert_eq!(items[1]["number"], 43);
        assert_eq!(items[1]["title"], "Bump deps");
        assert_eq!(items[1]["head"], "chore/bump");
        for item in items {
            for k in OMITTED_ON_A_LIST_ITEM {
                assert!(item.get(k).is_none(), "key '{k}' must not be on a list item");
            }
            assert!(item.get("raw").is_some(), "raw IS present on a list item");
        }
    });
}

/// E2E-NEW-724: every accepted `state` maps to the provider's own spelling, and
/// GitHub, which has no merged filter, filters its own answer.
#[test]
fn e2e_new_724_state_maps_to_each_providers_own_spelling() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/projects/acme%2Fapi/merge_requests", 200, json!([]));
        for state in ["open", "closed", "merged", "all"] {
            e.list(json!({"mount_id": "acme-lab", "state": state})).await.unwrap();
        }
        let gitlab_states: Vec<String> = e
            .api
            .calls()
            .iter()
            .map(|c| c.query.iter().find(|(k, _)| k == "state").unwrap().1.clone())
            .collect();
        assert_eq!(gitlab_states, vec!["opened", "closed", "merged", "all"]);

        e.api.clear();
        e.api.route("GET", "/repos/acme/api/pulls", 200, json!([]));
        for state in ["open", "closed", "all"] {
            e.list(json!({"mount_id": "acme-api", "state": state})).await.unwrap();
        }
        // `merged` is GitHub's `closed` plus a client side filter.
        let mut merged = gh_list_item(40);
        merged["state"] = json!("closed");
        merged["merged"] = json!(true);
        merged["merged_at"] = json!("2026-08-30T08:00:00Z");
        e.api.route("GET", "/repos/acme/api/pulls", 200, json!([gh_list_item(42), merged]));
        let out = e.list(json!({"mount_id": "acme-api", "state": "merged"})).await.unwrap();

        let github_states: Vec<String> = e
            .api
            .calls()
            .iter()
            .map(|c| c.query.iter().find(|(k, _)| k == "state").unwrap().1.clone())
            .collect();
        assert_eq!(github_states, vec!["open", "closed", "all", "closed"]);
        assert_eq!(out["count"], 1, "the unmerged item is filtered out");
        assert_eq!(out["pull_requests"][0]["number"], 40);
        assert_eq!(out["pull_requests"][0]["state"], "merged");
    });
}

/// E2E-NEW-725: an empty result is an empty array and the number 0.
#[test]
fn e2e_new_725_an_empty_result_is_an_empty_array_and_a_zero_count() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/pulls", 200, json!([]));
        e.api.route("GET", "/projects/acme%2Fapi/merge_requests", 200, json!([]));

        for mount in ["acme-api", "acme-lab"] {
            let out = e.list(json!({"mount_id": mount, "state": "open"})).await.unwrap();
            assert_eq!(out, json!({"pull_requests": [], "count": 0}));
            assert_eq!(out["count"], json!(0));
            assert!(out["count"].is_number(), "count is the number 0, never null");
        }
    });
}

/// E2E-NEW-726: an unsupported `state` is rejected before the network call.
#[test]
fn e2e_new_726_an_unsupported_state_is_rejected_before_any_call() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/pulls", 200, json!([]));

        let err = e.list(json!({"mount_id": "acme-api", "state": "draft"})).await.unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("state must be one of open, closed, merged, all"),
            "got: {}",
            err.message
        );
        assert!(e.api.calls().is_empty());
    });
}

/// E2E-NEW-727: a full page is followed by the next one, and a short page ends
/// the walk.
#[test]
fn e2e_new_727_a_full_page_is_followed_by_the_next_one() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let page1: Vec<Value> = (1..=100).map(gh_list_item).collect();
        let page2: Vec<Value> = (101..=150).map(gh_list_item).collect();
        e.api.route_q(
            "GET",
            "/repos/acme/api/pulls",
            "state=open&per_page=100",
            200,
            Value::Array(page1),
        );
        e.api.route_q(
            "GET",
            "/repos/acme/api/pulls",
            "state=open&per_page=100&page=2",
            200,
            Value::Array(page2),
        );

        let out = e.list(json!({"mount_id": "acme-api", "state": "open"})).await.unwrap();
        assert_eq!(out["count"], 150);

        let calls = e.api.calls();
        assert_eq!(calls.len(), 2, "the short second page ends the walk");
        assert!(calls[1].query.iter().any(|(k, v)| k == "page" && v == "2"));
        assert!(calls[0].query.iter().all(|(k, _)| k != "page"), "page 1 carries no page param");

        let numbers: Vec<u64> = out["pull_requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["number"].as_u64().unwrap())
            .collect();
        assert_eq!(numbers, (1..=150).collect::<Vec<u64>>(), "contiguous, in response order");
    });
}

/// E2E-NEW-730: a GitHub Enterprise item carries its own host.
#[test]
fn e2e_new_730_a_list_item_from_enterprise_carries_its_host() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/pulls", 200, json!([gh_list_item(42)]));

        let out = e.list(json!({"mount_id": "acme-ent", "state": "open"})).await.unwrap();
        assert_eq!(e.api.calls()[0].base_url, "https://github.ibm.com/api/v3");
        assert_eq!(out["count"], 1);
        assert_eq!(out["pull_requests"][0]["host"], "github.ibm.com");
        assert_eq!(keys_of(&out["pull_requests"][0]), LIST_KEYS);
    });
}

/// E2E-NEW-731: the same for a self hosted GitLab.
#[test]
fn e2e_new_731_a_list_item_from_self_hosted_gitlab_carries_its_host() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let mut item = gl_mr_42_open();
        for k in ["commits_count", "changes_count", "diff_stats_summary", "merge_status"] {
            item.as_object_mut().unwrap().remove(k);
        }
        e.api.route("GET", "/projects/acme%2Fapi/merge_requests", 200, json!([item]));

        let out = e.list(json!({"mount_id": "acme-self", "state": "open"})).await.unwrap();
        assert_eq!(e.api.calls()[0].base_url, "https://gitlab.example.test/api/v4");
        assert_eq!(out["count"], 1);
        assert_eq!(out["pull_requests"][0]["host"], "gitlab.example.test");
        assert_eq!(keys_of(&out["pull_requests"][0]), LIST_KEYS);
    });
}

/// E2E-NEW-776: a read needs `api` or `read_api` on GitLab, and the refusal is
/// `ERR_FORBIDDEN` (the credential is valid, merely too narrow), checked before
/// the network.
///
/// The message is the one US-023 froze in `scopes.rs`, prefixed by the tool
/// name; the assertions cover every part the story requires it to carry.
#[test]
fn e2e_new_776_a_read_scope_short_token_is_refused_before_the_network() {
    with_hosts_lock(async {
        let e = PrEnv::build(&[(
            "gitlab.com",
            "gitlab",
            GL_TOKEN,
            &["read_repository", "write_repository"],
        )])
        .await;

        let err = e.list(json!({"mount_id": "acme-lab", "state": "open"})).await.unwrap_err();
        assert_eq!(err.code, code::FORBIDDEN);
        assert_ne!(err.code, code::UNAUTHENTICATED);
        for part in [
            "git.pr_list",
            "host gitlab.com",
            "missing scope 'api' (or 'read_api')",
            "git.auth",
            "git.token_set",
        ] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(!err.message.contains("glpat"), "got: {}", err.message);
        assert!(e.api.calls().is_empty(), "checked before the network");
    });
}

/// `tracing_subscriber`'s `MakeWriter` over a shared buffer.
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ── US-027: git.pr_get and git.pr_diff ──────────────────────────────────────

/// The head sha E2E-NEW-732 asserts the check-runs route is built from.
const HEAD_SHA: &str = "9f1c2ab3d4e5f60718293a4b5c6d7e8f90a1b2c3";

/// GitHub's single pull request payload, with the head sha the check-runs route
/// is addressed by.
fn gh_pr_42_detail() -> Value {
    let mut v = gh_pr_42_open();
    v["head"]["sha"] = json!(HEAD_SHA);
    v
}

/// GitLab's single merge request payload. `head_pipeline` is where its check
/// state comes from, so no third call is needed on that provider.
fn gl_mr_42_detail() -> Value {
    let mut v = gl_mr_42_open();
    v["head_pipeline"] = json!({"id": 77, "status": "success", "sha": HEAD_SHA});
    v
}

const GH_PR: &str = "/repos/acme/api/pulls/42";
const GH_REVIEWS: &str = "/repos/acme/api/pulls/42/reviews";
const GL_MR: &str = "/projects/acme%2Fapi/merge_requests/42";
const GL_APPROVALS: &str = "/projects/acme%2Fapi/merge_requests/42/approvals";

fn gh_check_runs() -> String {
    format!("/repos/acme/api/commits/{HEAD_SHA}/check-runs")
}

/// Route the three GitHub endpoints `git.pr_get` consults, approved and green.
fn route_gh_get(e: &PrEnv) {
    e.api.route("GET", GH_PR, 200, gh_pr_42_detail());
    e.api.route(
        "GET",
        GH_REVIEWS,
        200,
        json!([{"user": {"login": "reviewer"}, "state": "APPROVED",
                "submitted_at": "2026-09-02T09:00:00Z"}]),
    );
    e.api.route(
        "GET",
        &gh_check_runs(),
        200,
        json!({"total_count": 1,
               "check_runs": [{"name": "ci", "status": "completed", "conclusion": "success"}]}),
    );
}

fn route_gl_get(e: &PrEnv) {
    e.api.route("GET", GL_MR, 200, gl_mr_42_detail());
    e.api.route(
        "GET",
        GL_APPROVALS,
        200,
        json!({"approved": true, "approvals_required": 1, "approvals_left": 0,
               "approved_by": [{"user": {"username": "reviewer"}}]}),
    );
}

/// E2E-NEW-732: GitHub `git.pr_get` issues exactly three calls, in order, and
/// the check-runs route is addressed by the head sha of the FIRST answer.
#[test]
fn e2e_new_732_pr_get_on_github_enriches_from_reviews_and_check_runs() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gh_get(&e);

        let out = e.get(json!({"mount_id": "acme-api", "pr_number": 42})).await.unwrap();

        assert_eq!(
            e.api.steps(),
            vec![
                ("GET".to_string(), GH_PR.to_string()),
                ("GET".to_string(), GH_REVIEWS.to_string()),
                ("GET".to_string(), gh_check_runs()),
            ]
        );
        assert_eq!(keys_of(&out), NORMALIZED_KEYS, "the full frozen model");
        assert_eq!(out["number"], 42);
        assert_eq!(out["review_state"], "approved");
        assert_eq!(out["checks_state"], "success");
        assert!(!serde_json::to_string(&out).unwrap().contains("ghp_"));
    });
}

/// E2E-NEW-733: GitLab needs two calls only: the pipeline state rides along on
/// the merge request payload.
#[test]
fn e2e_new_733_pr_get_on_gitlab_enriches_from_approvals_and_head_pipeline() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gl_get(&e);

        let out = e.get(json!({"mount_id": "acme-lab", "pr_number": 42})).await.unwrap();

        assert_eq!(
            e.api.steps(),
            vec![
                ("GET".to_string(), GL_MR.to_string()),
                ("GET".to_string(), GL_APPROVALS.to_string()),
            ]
        );
        assert_eq!(out["number"], 42);
        assert_eq!(out["checks_state"], "success", "from head_pipeline.status");
        assert_eq!(out["review_state"], "approved", "from approved:true");
        assert!(!serde_json::to_string(&out).unwrap().contains("glpat"));
    });
}

/// E2E-NEW-734: zero counts stay zero on both providers, and GitLab's absent
/// `diff_stats_summary` normalizes to 0 rather than null.
#[test]
fn e2e_new_734_zero_counts_normalize_to_zero_on_both_providers() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let mut gh = gh_pr_42_detail();
        for k in ["changed_files", "additions", "deletions", "commits"] {
            gh[k] = json!(0);
        }
        e.api.route("GET", GH_PR, 200, gh);
        e.api.route("GET", GH_REVIEWS, 200, json!([]));
        e.api.route("GET", &gh_check_runs(), 200, json!({"total_count": 0, "check_runs": []}));

        let mut gl = gl_mr_42_detail();
        gl["changes_count"] = json!("0");
        gl["commits_count"] = json!(0);
        gl.as_object_mut().unwrap().remove("diff_stats_summary");
        e.api.route("GET", GL_MR, 200, gl);
        e.api.route("GET", GL_APPROVALS, 200, json!({"approved": false, "approvals_left": 0}));

        for mount in ["acme-api", "acme-lab"] {
            let out = e.get(json!({"mount_id": mount, "pr_number": 42})).await.unwrap();
            for k in ["changed_files", "additions", "deletions", "commits"] {
                assert_eq!(out[k], json!(0), "{mount}: key '{k}'");
                assert!(!out[k].is_null(), "{mount}: key '{k}' must be 0, not null");
            }
        }
    });
}

/// E2E-NEW-741: FR-NEW-308, a failed check-runs sub-call fails the whole call
/// rather than degrading `checks_state` to `none`.
#[test]
fn e2e_new_741_a_failed_check_runs_sub_call_fails_pr_get() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", GH_PR, 200, gh_pr_42_detail());
        e.api.route("GET", GH_REVIEWS, 200, json!([]));
        e.api.route(
            "GET",
            &gh_check_runs(),
            403,
            json!({"message": "Resource not accessible by integration"}),
        );

        let err = e.get(json!({"mount_id": "acme-api", "pr_number": 42})).await.unwrap_err();
        assert_eq!(err.code, code::FORBIDDEN);
        for part in ["check runs", "Resource not accessible by integration"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(!err.message.contains("ghp_"), "got: {}", err.message);
        assert_eq!(e.api.calls().len(), 3);
    });
}

const UNIFIED_DIFF_SMALL: &str = "diff --git a/src/lib.rs b/src/lib.rs\n\
index 1111111..2222222 100644\n\
--- a/src/lib.rs\n\
+++ b/src/lib.rs\n\
@@ -1,2 +1,3 @@\n\
 pub mod git;\n\
+pub mod pr;\n";

/// E2E-NEW-742: GitHub's diff is the pull request route under a diff media
/// type, returned byte for byte.
#[test]
fn e2e_new_742_pr_diff_on_github_negotiates_the_diff_media_type() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route_text("GET", GH_PR, 200, UNIFIED_DIFF_SMALL, "text/plain");

        let out = e.diff(json!({"mount_id": "acme-api", "pr_number": 42})).await.unwrap();

        let calls = e.api.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].path, GH_PR);
        assert_eq!(calls[0].accept.as_deref(), Some("application/vnd.github.v3.diff"));
        assert_eq!(out["pr_number"], 42);
        assert_eq!(out["diff"].as_str().unwrap(), UNIFIED_DIFF_SMALL);
        assert_eq!(out["bytes"], UNIFIED_DIFF_SMALL.len());
        assert_eq!(out["truncated"], false);
    });
}

/// E2E-NEW-743: GitLab answers per file hunks with no headers, so the `diff
/// --git`, mode and `---`/`+++` lines are synthesized into a valid unified diff.
#[test]
fn e2e_new_743_pr_diff_on_gitlab_synthesizes_the_per_file_headers() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/merge_requests/42/changes",
            200,
            json!({"changes": [{"old_path": "src/tools/pr.rs", "new_path": "src/tools/pr.rs",
                               "new_file": true, "deleted_file": false, "renamed_file": false,
                               "diff": "@@ -0,0 +1,3 @@\n+pub fn register() {}\n"}]}),
        );

        let out = e.diff(json!({"mount_id": "acme-lab", "pr_number": 42})).await.unwrap();
        assert_eq!(
            out["diff"].as_str().unwrap(),
            "diff --git a/src/tools/pr.rs b/src/tools/pr.rs\n\
             new file mode 100644\n\
             --- /dev/null\n\
             +++ b/src/tools/pr.rs\n\
             @@ -0,0 +1,3 @@\n\
             +pub fn register() {}\n"
        );
        assert_eq!(out["truncated"], false);
    });
}

/// E2E-NEW-744: an empty change set is an empty diff on both providers, not an
/// error.
#[test]
fn e2e_new_744_an_empty_diff_is_empty_on_both_providers() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route_text("GET", GH_PR, 200, "", "text/plain");
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/merge_requests/42/changes",
            200,
            json!({"changes": []}),
        );

        for mount in ["acme-api", "acme-lab"] {
            let out = e.diff(json!({"mount_id": mount, "pr_number": 42})).await.unwrap();
            assert_eq!(out["diff"], "", "{mount}");
            assert_eq!(out["bytes"], 0, "{mount}");
            assert_eq!(out["truncated"], false, "{mount}");
        }
    });
}

/// E2E-NEW-745: past `git.max_pr_diff_mb` the answer is the prefix, explicitly
/// marked, with the remedy named.
#[test]
fn e2e_new_745_an_oversize_diff_is_truncated_at_the_configured_cap() {
    with_hosts_lock(async {
        let cap = 5 * 1024 * 1024;
        let e = PrEnv::build_with(&[("github.com", "github", GH_TOKEN, &["repo"])], Some(5)).await;
        let body = "+x".repeat(6 * 1024 * 1024);
        assert_eq!(body.len(), 12 * 1024 * 1024);
        e.api.route_text("GET", GH_PR, 200, &body, "text/plain");

        let out = e.diff(json!({"mount_id": "acme-api", "pr_number": 42})).await.unwrap();
        assert_eq!(out["truncated"], true);
        assert_eq!(out["bytes"], cap);
        let diff = out["diff"].as_str().unwrap();
        assert_eq!(diff.len(), cap);
        assert!(body.starts_with(diff), "the answer is a prefix of the source body");
        assert!(
            out["note"].as_str().unwrap().contains(
                "diff truncated at 5 MiB; fetch the branch with git.remote_fetch for the full \
                 change"
            ),
            "got: {}",
            out["note"]
        );
    });
}

/// E2E-NEW-746: the diff is bytes, not characters: CRLF pairs survive and
/// `bytes` counts bytes.
#[test]
fn e2e_new_746_a_unicode_crlf_diff_is_returned_byte_for_byte() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let body = "+// été ✅ 日本語\r\n+second\r\n";
        e.api.route_text("GET", GH_PR, 200, body, "text/plain");

        let out = e.diff(json!({"mount_id": "acme-api", "pr_number": 42})).await.unwrap();
        assert_eq!(out["diff"].as_str().unwrap(), body);
        // `str::len` IS the byte length, which is the point of the assertion:
        // this body has more bytes than characters.
        assert_eq!(out["bytes"], body.len());
        assert_ne!(body.len(), body.chars().count(), "bytes and chars differ here");
    });
}

/// E2E-NEW-748: a 200 that is not a diff at all is an error naming the content
/// type, and the body is never dumped into the message.
#[test]
fn e2e_new_748_an_html_answer_under_a_200_is_refused() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route_text(
            "GET",
            GH_PR,
            200,
            "<!DOCTYPE html><html><body>login</body></html>",
            "text/html; charset=utf-8",
        );

        let err = e.diff(json!({"mount_id": "acme-api", "pr_number": 42})).await.unwrap_err();
        assert_eq!(err.code, code::INTERNAL_ERROR);
        assert!(
            err.message
                .contains("expected a unified diff from github.com, got content-type 'text/html'"),
            "got: {}",
            err.message
        );
        assert!(!err.message.contains("<!DOCTYPE"), "no body dumping: {}", err.message);
    });
}

/// E2E-NEW-749: a binary change carries no hunk, so only the `diff --git` line
/// is synthesized and GitLab's own wording is kept.
#[test]
fn e2e_new_749_a_binary_gitlab_change_keeps_its_own_wording() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/merge_requests/42/changes",
            200,
            json!({"changes": [{"old_path": "logo.png", "new_path": "logo.png",
                               "new_file": false, "deleted_file": false, "renamed_file": false,
                               "diff": "Binary files a/logo.png and b/logo.png differ\n"}]}),
        );

        let out = e.diff(json!({"mount_id": "acme-lab", "pr_number": 42})).await.unwrap();
        assert_eq!(
            out["diff"].as_str().unwrap(),
            "diff --git a/logo.png b/logo.png\n\
             Binary files a/logo.png and b/logo.png differ\n"
        );
    });
}

/// E2E-NEW-778: a read only GitLab scope set is enough for `git.pr_get`.
#[test]
fn e2e_new_778_pr_get_accepts_a_read_only_gitlab_scope_set() {
    with_hosts_lock(async {
        let e =
            PrEnv::build(&[("gitlab.com", "gitlab", GL_TOKEN, &["read_api", "read_repository"])])
                .await;
        route_gl_get(&e);

        let out = e.get(json!({"mount_id": "acme-lab", "pr_number": 42})).await.unwrap();
        assert_eq!(out["number"], 42);
        assert_eq!(e.api.calls().len(), 2);
    });
}

/// E2E-NEW-878: FR-NEW-308 on GitLab: a 500 from approvals fails the call, and
/// no partial model is produced.
#[test]
fn e2e_new_878_a_failed_gitlab_approvals_sub_call_fails_pr_get() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", GL_MR, 200, gl_mr_42_detail());
        e.api.route("GET", GL_APPROVALS, 500, json!({"message": "500 Internal Server Error"}));

        let out = e.get(json!({"mount_id": "acme-lab", "pr_number": 42})).await;
        assert!(out.is_err(), "no Ok value is produced at all");
        let err = out.unwrap_err();
        for part in ["500", "approvals"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(!err.message.contains("glpat"), "got: {}", err.message);
        assert!(!err.message.contains(GL_TOKEN));
        assert_eq!(
            e.api.steps(),
            vec![
                ("GET".to_string(), GL_MR.to_string()),
                ("GET".to_string(), GL_APPROVALS.to_string()),
            ]
        );
    });
}

/// E2E-NEW-879: an observed empty check set really is `none`, and the contrast
/// with a failing sub-call is asserted in the same test.
#[test]
fn e2e_new_879_an_empty_check_set_is_none_while_a_failure_is_an_error() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", GH_PR, 200, gh_pr_42_detail());
        e.api.route("GET", GH_REVIEWS, 200, json!([]));
        e.api.route("GET", &gh_check_runs(), 200, json!({"total_count": 0, "check_runs": []}));

        let out = e.get(json!({"mount_id": "acme-api", "pr_number": 42})).await.unwrap();
        assert_eq!(out["checks_state"], "none");
        assert_eq!(out["review_state"], "none");
        assert_eq!(e.api.calls().len(), 3, "none is an observed empty set");

        e.api.clear();
        e.api.route("GET", &gh_check_runs(), 500, json!({"message": "boom"}));
        let err = e.get(json!({"mount_id": "acme-api", "pr_number": 42})).await.unwrap_err();
        assert_eq!(err.code, code::INTERNAL_ERROR);
        assert!(err.message.contains("check runs"), "got: {}", err.message);
        assert_eq!(e.api.calls().len(), 3);
    });
}

// ── US-028: git.pr_merge ────────────────────────────────────────────────────

const GH_MERGE: &str = "/repos/acme/api/pulls/42/merge";
const GL_MERGE: &str = "/projects/acme%2Fapi/merge_requests/42/merge";
const GL_REBASE: &str = "/projects/acme%2Fapi/merge_requests/42/rebase";
const MERGE_SHA: &str = "3c0ffee1234567890abcdef1234567890abcdef1";

/// GitHub's merge answer: a merge result, not a pull request payload.
fn gh_merge_ok() -> Value {
    json!({"sha": MERGE_SHA, "merged": true, "message": "Pull Request successfully merged"})
}

/// GitLab answers a merge with the merge request itself, already in state
/// `merged`.
fn gl_mr_42_merged() -> Value {
    let mut v = gl_mr_42_detail();
    v["state"] = json!("merged");
    v
}

fn step(method: &str, path: &str) -> (String, String) {
    (method.to_string(), path.to_string())
}

/// The GitHub pre-read plus a canned merge answer, the two routes every GitHub
/// merge test needs.
fn route_gh_merge(e: &PrEnv, status: u16, answer: Value) {
    e.api.route("GET", GH_PR, 200, gh_pr_42_detail());
    e.api.route("PUT", GH_MERGE, status, answer);
}

fn route_gl_merge(e: &PrEnv, status: u16, answer: Value) {
    e.api.route("GET", GL_MR, 200, gl_mr_42_detail());
    e.api.route("PUT", GL_MERGE, status, answer);
}

/// E2E-NEW-750: FR-NEW-310, a GitHub merge sends exactly `{"merge_method":
/// "merge"}` and reports the provider's merge result.
#[test]
fn e2e_new_750_a_github_merge_sends_the_merge_method_and_reports_merged() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gh_merge(&e, 200, gh_merge_ok());

        let out = e
            .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "merge"}))
            .await
            .unwrap();

        let puts: Vec<_> = e.api.calls().into_iter().filter(|c| c.method == "PUT").collect();
        assert_eq!(puts.len(), 1);
        assert_eq!(puts[0].path, GH_MERGE);
        assert_eq!(puts[0].body, Some(json!({"merge_method": "merge"})));
        assert_eq!(out["state"], "merged");
        assert_eq!(out["raw"]["sha"], MERGE_SHA);
        // FR-NEW-310: the merge happened on the provider only, and the answer
        // says so rather than letting a caller assume a local ref moved.
        assert!(
            out["note"].as_str().unwrap_or_default().contains("git.remote_fetch"),
            "got: {}",
            out["note"]
        );
        assert!(!serde_json::to_string(&out).unwrap().contains("ghp_"));
    });
}

/// E2E-NEW-751: the same call with `squash` maps to GitHub's own spelling.
#[test]
fn e2e_new_751_a_github_squash_merge_sends_the_squash_merge_method() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gh_merge(&e, 200, gh_merge_ok());

        let out = e
            .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "squash"}))
            .await
            .unwrap();

        let puts: Vec<_> = e.api.calls().into_iter().filter(|c| c.method == "PUT").collect();
        assert_eq!(puts[0].body, Some(json!({"merge_method": "squash"})));
        assert_eq!(out["state"], "merged");
    });
}

/// E2E-NEW-752: GitHub rebases inside the merge call, so a rebase costs exactly
/// one PUT and no pre-rebase round trip.
#[test]
fn e2e_new_752_a_github_rebase_merge_costs_exactly_one_put() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gh_merge(&e, 200, gh_merge_ok());

        let out = e
            .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "rebase"}))
            .await
            .unwrap();

        let puts: Vec<_> = e.api.calls().into_iter().filter(|c| c.method == "PUT").collect();
        assert_eq!(puts.len(), 1, "GitHub needs no pre-rebase call");
        assert_eq!(puts[0].body, Some(json!({"merge_method": "rebase"})));
        assert_eq!(out["state"], "merged");
    });
}

/// E2E-NEW-753: GitLab has no merge method: the strategy becomes its `squash`
/// flag, false for a plain merge.
#[test]
fn e2e_new_753_a_gitlab_merge_sends_squash_false() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gl_merge(&e, 200, gl_mr_42_merged());

        let out = e
            .merge(json!({"mount_id": "acme-lab", "pr_number": 42, "strategy": "merge"}))
            .await
            .unwrap();

        let puts: Vec<_> = e.api.calls().into_iter().filter(|c| c.method == "PUT").collect();
        assert_eq!(puts.len(), 1);
        assert_eq!(puts[0].path, GL_MERGE);
        assert_eq!(puts[0].body, Some(json!({"squash": false})));
        assert_eq!(out["state"], "merged");
        assert!(!serde_json::to_string(&out).unwrap().contains("glpat"));
    });
}

/// E2E-NEW-754: the same call with `squash` flips that one flag.
#[test]
fn e2e_new_754_a_gitlab_squash_merge_sends_squash_true() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gl_merge(&e, 200, gl_mr_42_merged());

        let out = e
            .merge(json!({"mount_id": "acme-lab", "pr_number": 42, "strategy": "squash"}))
            .await
            .unwrap();

        let puts: Vec<_> = e.api.calls().into_iter().filter(|c| c.method == "PUT").collect();
        assert_eq!(puts[0].body, Some(json!({"squash": true})));
        assert_eq!(out["state"], "merged");
    });
}

/// E2E-NEW-755: GitLab rebases on its own endpoint, asynchronously, so the
/// strategy is a three step choreography: rebase, wait for it, then merge.
///
/// Deviation from the story's literal call list: the pre-read of FR-NEW-311
/// runs first on BOTH providers (it is what E2E-NEW-762 requires), so the
/// assertion here is that the three rebase steps follow it in exactly that
/// order.
#[test]
fn e2e_new_755_a_gitlab_rebase_merge_rebases_waits_then_merges() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gl_merge(&e, 200, gl_mr_42_merged());
        e.api.route("POST", GL_REBASE, 202, json!({"rebase_in_progress": true}));
        e.api.route_q(
            "GET",
            GL_MR,
            "include_rebase_in_progress=true",
            200,
            json!({"rebase_in_progress": false, "merge_error": null}),
        );

        let out = e
            .merge(json!({"mount_id": "acme-lab", "pr_number": 42, "strategy": "rebase"}))
            .await
            .unwrap();

        assert_eq!(
            e.api.steps(),
            vec![
                step("GET", GL_MR),
                step("POST", GL_REBASE),
                step("GET", GL_MR),
                step("PUT", GL_MERGE),
            ]
        );
        assert_eq!(
            e.api.calls()[2].query,
            vec![("include_rebase_in_progress".to_string(), "true".to_string())]
        );
        let puts: Vec<_> = e.api.calls().into_iter().filter(|c| c.method == "PUT").collect();
        assert_eq!(puts[0].body, Some(json!({"squash": false})));
        assert_eq!(out["state"], "merged");
    });
}

/// E2E-NEW-756: an unsupported strategy is refused before anything is resolved,
/// so it costs no call at all.
#[test]
fn e2e_new_756_an_unsupported_strategy_is_refused_before_the_network() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gh_merge(&e, 200, gh_merge_ok());

        let err = e
            .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "fast-forward"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("strategy must be one of merge, squash, rebase"),
            "got: {}",
            err.message
        );
        assert!(e.api.calls().is_empty(), "no call at all");
    });
}

/// E2E-NEW-757: FR-NEW-311, a strategy disabled in the repository settings is
/// the provider's refusal, reported as such, and never retried with another
/// strategy behind the caller's back.
#[test]
fn e2e_new_757_a_disabled_strategy_is_surfaced_and_never_retried() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gh_merge(
            &e,
            405,
            json!({"message": "Merge commits are not allowed on this repository.",
                   "documentation_url": "https://docs.github.com/rest/pulls/pulls#merge"}),
        );

        let err = e
            .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "merge"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::INVALID_ARGUMENT);
        for part in ["Merge commits are not allowed on this repository.", "strategy 'merge'"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        let puts = e.api.calls().into_iter().filter(|c| c.method == "PUT").count();
        assert_eq!(puts, 1, "no silent retry with another strategy");
    });
}

/// E2E-NEW-758: a failing required check is the provider's own sentence, kept
/// verbatim.
#[test]
fn e2e_new_758_a_missing_required_check_is_surfaced_verbatim() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gh_merge(
            &e,
            405,
            json!({"message": "Required status check \"ci/build\" is expected."}),
        );

        let err = e
            .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "merge"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("Required status check \"ci/build\" is expected."),
            "got: {}",
            err.message
        );
    });
}

/// E2E-NEW-759: GitLab refuses an unmergeable merge request with a bare 405,
/// and that bare sentence plus the host is what the caller gets.
#[test]
fn e2e_new_759_a_gitlab_unmergeable_refusal_is_surfaced() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let mut mr = gl_mr_42_detail();
        mr["merge_status"] = json!("cannot_be_merged");
        mr["head_pipeline"] = json!({"id": 77, "status": "failed"});
        e.api.route("GET", GL_MR, 200, mr);
        e.api.route("PUT", GL_MERGE, 405, json!({"message": "405 Method Not Allowed"}));

        let err = e
            .merge(json!({"mount_id": "acme-lab", "pr_number": 42, "strategy": "merge"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::INVALID_ARGUMENT);
        for part in ["405 Method Not Allowed", "gitlab.com"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(!err.message.contains("glpat"), "got: {}", err.message);
    });
}

/// E2E-NEW-760: a head branch that moved under the merge is a 409, surfaced,
/// not swallowed into a success.
#[test]
fn e2e_new_760_a_modified_head_branch_is_surfaced() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gh_merge(
            &e,
            409,
            json!({"message": "Head branch was modified. Review and try the merge again.",
                   "sha": MERGE_SHA}),
        );

        let out =
            e.merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "merge"})).await;

        assert!(out.is_err(), "a refused merge produces no success shape at all");
        let err = out.unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("Head branch was modified. Review and try the merge again."),
            "got: {}",
            err.message
        );
    });
}

/// E2E-NEW-761: a protected branch refusal is `ERR_FORBIDDEN`, with the
/// provider's own wording and no credential in it.
#[test]
fn e2e_new_761_a_protected_branch_refusal_is_forbidden() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        route_gh_merge(
            &e,
            403,
            json!({"message": "4 of 4 required status checks are expected. Protected branch \
                               rules not met."}),
        );

        let err = e
            .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "squash"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::FORBIDDEN);
        assert!(err.message.contains("Protected branch rules not met."), "got: {}", err.message);
        assert!(!err.message.contains("ghp_"), "got: {}", err.message);
    });
}

/// E2E-NEW-762: an already merged pull request is refused by the pre-read, so
/// the merge request is never sent at all.
#[test]
fn e2e_new_762_an_already_merged_pull_request_short_circuits_the_put() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let mut merged = gh_pr_42_detail();
        merged["merged"] = json!(true);
        merged["merged_at"] = json!("2026-09-03T08:00:00Z");
        merged["state"] = json!("closed");
        e.api.route("GET", GH_PR, 200, merged);
        e.api.route("PUT", GH_MERGE, 405, json!({"message": "Pull Request is not mergeable"}));

        let err = e
            .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "merge"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(err.message.contains("pull request 42 is already merged"), "got: {}", err.message);
        assert_eq!(e.api.steps(), vec![step("GET", GH_PR)], "the PUT was never sent");
    });
}

/// E2E-NEW-779: FR-NEW-332, merging is a write, so a GitLab read scope set is
/// refused before the network and `read_api` is never offered as the remedy.
#[test]
fn e2e_new_779_a_gitlab_read_scope_set_cannot_merge() {
    with_hosts_lock(async {
        let e =
            PrEnv::build(&[("gitlab.com", "gitlab", GL_TOKEN, &["read_api", "read_repository"])])
                .await;

        let err = e
            .merge(json!({"mount_id": "acme-lab", "pr_number": 42, "strategy": "squash"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::FORBIDDEN);
        for part in ["git.pr_merge", "host gitlab.com", "missing scope 'api'"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(
            !err.message.contains("read_api"),
            "a read scope is no remedy for a merge: {}",
            err.message
        );
        assert!(!err.message.contains("glpat"), "got: {}", err.message);
        assert!(e.api.calls().is_empty(), "checked before the network");
    });
}

/// E2E-NEW-780: the same gate on GitHub, where the scope to request is `repo`.
#[test]
fn e2e_new_780_a_public_repo_github_token_cannot_merge() {
    with_hosts_lock(async {
        let e = PrEnv::build(&[("github.com", "github", GH_TOKEN, &["public_repo"])]).await;

        let err = e
            .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "merge"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::FORBIDDEN);
        for part in ["git.pr_merge", "token for host github.com is missing scope 'repo'"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(!err.message.contains("ghp_"), "got: {}", err.message);
        assert!(e.api.calls().is_empty(), "checked before the network");
    });
}

// ── US-029: git.pr_review ───────────────────────────────────────────────────

const GH_REVIEW: &str = "/repos/acme/api/pulls/42/reviews";
const GL_APPROVE: &str = "/projects/acme%2Fapi/merge_requests/42/approve";
const GL_UNAPPROVE: &str = "/projects/acme%2Fapi/merge_requests/42/unapprove";
const GL_NOTES: &str = "/projects/acme%2Fapi/merge_requests/42/notes";

const CHANGES_BODY: &str = "Please split the merge logic out of the tool layer.";

/// The six keys a review result carries, in their frozen order. A review is not
/// a pull request read: it reports what was submitted and what the review state
/// became, and inventing the twenty one pull request keys from a review payload
/// would put zeros where the provider said nothing.
const REVIEW_KEYS: [&str; 6] = ["pr_number", "provider", "host", "verdict", "review_state", "raw"];

/// GitLab answers its approvals endpoint with the approval set, which is where
/// an approve verdict reads its resulting state back from.
fn gl_approvals_approved() -> Value {
    json!({"approved": true, "approvals_required": 1, "approvals_left": 0,
           "approved_by": [{"user": {"username": "dev"}}]})
}

/// E2E-NEW-763: FR-NEW-312, a GitHub approval is one POST carrying exactly
/// `{"event":"APPROVE"}`, with no `body` key at all when none was supplied.
#[test]
fn e2e_new_763_a_github_approval_posts_the_approve_event_only() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "POST",
            GH_REVIEW,
            200,
            json!({"id": 7001, "state": "APPROVED", "user": {"login": "dev"}, "body": ""}),
        );

        let out =
            e.review(json!({"mount_id": "acme-api", "pr_number": 42, "verdict": "approve"})).await;
        let out = out.unwrap();

        assert_eq!(e.api.steps(), vec![step("POST", GH_REVIEW)]);
        assert_eq!(e.api.calls()[0].body, Some(json!({"event": "APPROVE"})));
        assert_eq!(keys_of(&out), REVIEW_KEYS);
        assert_eq!(out["review_state"], "approved");
        assert_eq!(out["verdict"], "approve");
        assert_eq!(out["pr_number"], 42);
        assert!(!serde_json::to_string(&out).unwrap().contains("ghp_"));
    });
}

/// E2E-NEW-764: the body rides along for `request_changes`, and the provider's
/// own answered state is what the result reports.
#[test]
fn e2e_new_764_a_github_request_changes_carries_the_body() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("POST", GH_REVIEW, 200, json!({"id": 7002, "state": "CHANGES_REQUESTED"}));

        let out = e
            .review(json!({"mount_id": "acme-api", "pr_number": 42,
                           "verdict": "request_changes", "body": CHANGES_BODY}))
            .await
            .unwrap();

        assert_eq!(
            e.api.calls()[0].body,
            Some(json!({"event": "REQUEST_CHANGES", "body": CHANGES_BODY}))
        );
        assert_eq!(out["review_state"], "changes_requested");
    });
}

/// E2E-NEW-765: a comment is not an approval, so the review state it leaves
/// behind is `review_required`: someone looked, nobody decided.
#[test]
fn e2e_new_765_a_github_comment_leaves_review_required() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("POST", GH_REVIEW, 200, json!({"id": 7003, "state": "COMMENTED"}));

        let out = e
            .review(json!({"mount_id": "acme-api", "pr_number": 42, "verdict": "comment",
                           "body": "Nit: typo in the doc comment."}))
            .await
            .unwrap();

        assert_eq!(
            e.api.calls()[0].body,
            Some(json!({"event": "COMMENT", "body": "Nit: typo in the doc comment."}))
        );
        assert_eq!(out["review_state"], "review_required");
    });
}

/// E2E-NEW-766: GitLab approves on its own endpoint, not by posting a review,
/// and the resulting state is read back from the approvals endpoint because the
/// approve answer carries no approval information at all.
#[test]
fn e2e_new_766_a_gitlab_approval_uses_the_approve_endpoint() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("POST", GL_APPROVE, 201, json!({"id": 42, "iid": 42, "state": "opened"}));
        e.api.route("GET", GL_APPROVALS, 200, gl_approvals_approved());

        let out = e
            .review(json!({"mount_id": "acme-lab", "pr_number": 42, "verdict": "approve"}))
            .await
            .unwrap();

        let posts: Vec<_> = e.api.calls().into_iter().filter(|c| c.method == "POST").collect();
        assert_eq!(posts.len(), 1, "exactly one POST, to the approve endpoint");
        assert_eq!(posts[0].path, GL_APPROVE);
        assert_eq!(posts[0].body, Some(json!({})));
        assert_eq!(e.api.steps(), vec![step("POST", GL_APPROVE), step("GET", GL_APPROVALS)]);
        assert_eq!(out["review_state"], "approved");
        assert_eq!(out["provider"], "gitlab");
        assert!(!serde_json::to_string(&out).unwrap().contains("glpat"));
    });
}

/// E2E-NEW-767: GitLab has no "changes requested" verdict. Requesting changes
/// is therefore withdrawing any approval, then leaving the note, in that order:
/// the other order would leave the merge request momentarily approved with a
/// blocking note already on it.
#[test]
fn e2e_new_767_a_gitlab_request_changes_unapproves_then_notes() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("POST", GL_UNAPPROVE, 201, json!({}));
        e.api.route("POST", GL_NOTES, 201, json!({"id": 9001, "body": CHANGES_BODY}));

        let out = e
            .review(json!({"mount_id": "acme-lab", "pr_number": 42,
                           "verdict": "request_changes", "body": CHANGES_BODY}))
            .await
            .unwrap();

        assert_eq!(e.api.steps(), vec![step("POST", GL_UNAPPROVE), step("POST", GL_NOTES)]);
        let notes = &e.api.calls()[1];
        assert_eq!(notes.body, Some(json!({"body": CHANGES_BODY})));
        assert_eq!(out["review_state"], "changes_requested", "the normalization under test");
        assert_eq!(out["raw"]["id"], 9001);

        // GitLab answers 404 when there was no approval to withdraw. Nothing to
        // withdraw is not a failure, so the note still goes out.
        e.api.clear();
        e.api.route("POST", GL_UNAPPROVE, 404, json!({"message": "404 Not found"}));
        let out = e
            .review(json!({"mount_id": "acme-lab", "pr_number": 42,
                           "verdict": "request_changes", "body": CHANGES_BODY}))
            .await
            .unwrap();
        assert_eq!(e.api.steps(), vec![step("POST", GL_UNAPPROVE), step("POST", GL_NOTES)]);
        assert_eq!(out["review_state"], "changes_requested");
    });
}

/// E2E-NEW-768: a GitLab comment touches no approval at all, so it costs one
/// call and neither approves nor unapproves anything.
#[test]
fn e2e_new_768_a_gitlab_comment_is_one_note_and_nothing_else() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("POST", GL_NOTES, 201, json!({"id": 9002, "body": "Nit: typo."}));

        let out = e
            .review(json!({"mount_id": "acme-lab", "pr_number": 42, "verdict": "comment",
                           "body": "Nit: typo."}))
            .await
            .unwrap();

        assert_eq!(e.api.steps(), vec![step("POST", GL_NOTES)]);
        assert_eq!(out["review_state"], "review_required");
    });
}

/// E2E-NEW-769: an unsupported verdict is a pure argument check, so it costs no
/// lookup and no call at all.
#[test]
fn e2e_new_769_an_unsupported_verdict_is_refused_before_any_call() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;

        let err = e
            .review(json!({"mount_id": "acme-api", "pr_number": 42, "verdict": "lgtm"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("verdict must be one of approve, request_changes, comment"),
            "got: {}",
            err.message
        );
        assert!(e.api.calls().is_empty(), "refused before the network");
    });
}

/// E2E-NEW-770: a verdict that carries no reasoning is useless to the author,
/// so `request_changes` and `comment` require a body, and whitespace is not one.
#[test]
fn e2e_new_770_request_changes_requires_a_non_empty_body() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;

        for args in [
            json!({"mount_id": "acme-api", "pr_number": 42, "verdict": "request_changes"}),
            json!({"mount_id": "acme-api", "pr_number": 42, "verdict": "request_changes",
                   "body": "   "}),
        ] {
            let err = e.review(args).await.unwrap_err();
            assert_eq!(err.code, code::INVALID_ARGUMENT);
            assert!(
                err.message.contains("verdict 'request_changes' requires a non-empty body"),
                "got: {}",
                err.message
            );
            assert!(e.api.calls().is_empty(), "refused before the network");
        }
    });
}

/// E2E-NEW-771: GitHub refuses a self approval with a 422 whose reason lives in
/// its `errors` array, not in `message`. That sentence is what the caller needs,
/// so it is surfaced.
#[test]
fn e2e_new_771_a_github_self_approval_is_surfaced_verbatim() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "POST",
            GH_REVIEW,
            422,
            json!({"message": "Unprocessable Entity",
                   "errors": ["Can not approve your own pull request"]}),
        );

        let err = e
            .review(json!({"mount_id": "acme-api", "pr_number": 42, "verdict": "approve"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("Can not approve your own pull request"),
            "got: {}",
            err.message
        );
        assert!(!err.message.contains("ghp_"), "got: {}", err.message);
    });
}

/// E2E-NEW-772: GitLab refuses the same thing with two different statuses, and
/// they are NOT the same answer: a 403 is a rule the caller broke, a 401 means
/// the credential itself is no longer accepted and the remedy must be named.
#[test]
fn e2e_new_772_the_gitlab_self_approval_refusals_keep_their_own_statuses() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "POST",
            GL_APPROVE,
            403,
            json!({"message": "Members can not approve their own merge request"}),
        );

        let err = e
            .review(json!({"mount_id": "acme-lab", "pr_number": 42, "verdict": "approve"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::FORBIDDEN);
        assert!(
            err.message.contains("Members can not approve their own merge request"),
            "got: {}",
            err.message
        );
        assert!(!err.message.contains("glpat_"), "got: {}", err.message);

        e.api.clear();
        e.api.route("POST", GL_APPROVE, 401, json!({"message": "401 Unauthorized"}));
        let err = e
            .review(json!({"mount_id": "acme-lab", "pr_number": 42, "verdict": "approve"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::UNAUTHENTICATED);
        for part in ["gitlab.com", "re-authenticate with git.auth"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert!(!err.message.contains("glpat_"), "got: {}", err.message);
    });
}

/// E2E-NEW-728: FR-NEW-313, a review on a pull request the provider does not
/// know names the repository, the host and the provider's own wording.
#[test]
fn e2e_new_728_a_review_on_an_unknown_pull_request_is_not_found() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "POST",
            GH_REVIEW,
            404,
            json!({"message": "Not Found",
                   "documentation_url": "https://docs.github.com/rest"}),
        );

        let err = e
            .review(json!({"mount_id": "acme-api", "pr_number": 42, "verdict": "approve"}))
            .await
            .unwrap_err();

        assert_eq!(err.code, code::NOT_FOUND);
        for part in ["acme/api", "github.com", "Not Found"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
    });
}

/// E2E-NEW-738: FR-NEW-313, an unknown number on GitHub fails on the first read
/// and fans out to nothing: no reviews call, no check runs call.
#[test]
fn e2e_new_738_an_unknown_github_number_costs_exactly_one_call() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("GET", "/repos/acme/api/pulls/9999", 404, json!({"message": "Not Found"}));

        let err = e.get(json!({"mount_id": "acme-api", "pr_number": 9999})).await.unwrap_err();

        assert_eq!(err.code, code::NOT_FOUND);
        for part in ["pull request 9999", "acme/api"] {
            assert!(err.message.contains(part), "missing '{part}' in: {}", err.message);
        }
        assert_eq!(e.api.calls().len(), 1, "no fan-out after a 404");
    });
}

/// E2E-NEW-739: the GitLab mirror, which names a merge request because that is
/// what GitLab calls it.
#[test]
fn e2e_new_739_an_unknown_gitlab_number_costs_exactly_one_call() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route(
            "GET",
            "/projects/acme%2Fapi/merge_requests/9999",
            404,
            json!({"message": "404 Not found"}),
        );

        let err = e.get(json!({"mount_id": "acme-lab", "pr_number": 9999})).await.unwrap_err();

        assert_eq!(err.code, code::NOT_FOUND);
        assert!(err.message.contains("merge request 9999"), "got: {}", err.message);
        assert_eq!(e.api.calls().len(), 1, "no fan-out after a 404");
    });
}

/// E2E-NEW-740: a zero or negative number names no pull request on either
/// provider, so it is refused before anything is looked up.
#[test]
fn e2e_new_740_a_non_positive_number_is_refused_before_any_call() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;

        for n in [0, -1] {
            let err = e
                .review(json!({"mount_id": "acme-api", "pr_number": n, "verdict": "approve"}))
                .await
                .unwrap_err();
            assert_eq!(err.code, code::INVALID_ARGUMENT);
            assert!(
                err.message.contains("pr_number must be a positive integer"),
                "got: {}",
                err.message
            );
            assert!(e.api.calls().is_empty(), "refused before the network");
        }
    });
}

/// E2E-NEW-747: FR-NEW-313, both providers name the number they could not find.
#[test]
fn e2e_new_747_a_review_404_names_the_number_on_both_providers() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        e.api.route("POST", GH_REVIEW, 404, json!({"message": "Not Found"}));
        e.api.route("POST", GL_APPROVE, 404, json!({"message": "404 Not found"}));

        for mount in ["acme-api", "acme-lab"] {
            let err = e
                .review(json!({"mount_id": mount, "pr_number": 42, "verdict": "approve"}))
                .await
                .unwrap_err();
            assert_eq!(err.code, code::NOT_FOUND, "on {mount}");
            assert!(err.message.contains("42"), "got: {}", err.message);
        }
    });
}

// ── the two end to end flows ────────────────────────────────────────────────

/// Route every GitHub endpoint the create -> get -> review -> merge flow walks,
/// then walk it, returning the four results in order.
///
/// The reviews route answers one COMMENTED review rather than the empty array
/// the story's route queue names, because the assertion the story makes on the
/// first `pr_get` is `review_state == "review_required"`, and an empty review
/// set is `none` (nobody looked) by FR-NEW-307. The payload is chosen to match
/// the asserted state rather than the other way round.
async fn github_review_flow(e: &PrEnv) -> Vec<Value> {
    e.api.route(
        "GET",
        "/repos/acme/api/branches/feature/pr-tools",
        200,
        json!({"name": "feature/pr-tools"}),
    );
    e.api.route("POST", "/repos/acme/api/pulls", 201, gh_pr_42_detail());
    e.api.route("GET", GH_PR, 200, gh_pr_42_detail());
    e.api.route(
        "GET",
        GH_REVIEWS,
        200,
        json!([{"user": {"login": "reviewer"}, "state": "COMMENTED"}]),
    );
    e.api.route(
        "GET",
        &gh_check_runs(),
        200,
        json!({"check_runs": [{"name": "ci", "status": "completed", "conclusion": "success"}]}),
    );
    e.api.route("POST", GH_REVIEW, 200, json!({"id": 7001, "state": "APPROVED"}));
    e.api.route("PUT", GH_MERGE, 200, gh_merge_ok());

    let created = e
        .create(json!({"mount_id": "acme-api", "base": "main", "head": "feature/pr-tools",
                       "title": "Add PR tools"}))
        .await
        .unwrap();
    let read = e.get(json!({"mount_id": "acme-api", "pr_number": 42})).await.unwrap();
    let reviewed = e
        .review(json!({"mount_id": "acme-api", "pr_number": 42, "verdict": "approve"}))
        .await
        .unwrap();
    let merged = e
        .merge(json!({"mount_id": "acme-api", "pr_number": 42, "strategy": "squash"}))
        .await
        .unwrap();
    vec![created, read, reviewed, merged]
}

/// The GitLab mirror of [`github_review_flow`], same four tools, same order.
async fn gitlab_review_flow(e: &PrEnv) -> Vec<Value> {
    e.api.route(
        "GET",
        "/projects/acme%2Fapi/repository/branches/feature/pr-tools",
        200,
        json!({"name": "feature/pr-tools"}),
    );
    e.api.route("POST", "/projects/acme%2Fapi/merge_requests", 201, gl_mr_42_detail());
    e.api.route("GET", GL_MR, 200, gl_mr_42_detail());
    e.api.route(
        "GET",
        GL_APPROVALS,
        200,
        json!({"approved": false, "approvals_required": 1, "approvals_left": 1}),
    );
    e.api.route("POST", GL_APPROVE, 201, json!({"id": 42, "iid": 42, "state": "opened"}));
    e.api.route("PUT", GL_MERGE, 200, gl_mr_42_merged());

    let created = e
        .create(json!({"mount_id": "acme-lab", "base": "main", "head": "feature/pr-tools",
                       "title": "Add PR tools"}))
        .await
        .unwrap();
    let read = e.get(json!({"mount_id": "acme-lab", "pr_number": 42})).await.unwrap();
    // The approve verdict reads the approvals back, so that route now answers
    // the approved set.
    e.api.route("GET", GL_APPROVALS, 200, gl_approvals_approved());
    let reviewed = e
        .review(json!({"mount_id": "acme-lab", "pr_number": 42, "verdict": "approve"}))
        .await
        .unwrap();
    let merged = e
        .merge(json!({"mount_id": "acme-lab", "pr_number": 42, "strategy": "squash"}))
        .await
        .unwrap();
    vec![created, read, reviewed, merged]
}

/// E2E-NEW-796: the whole GitHub flow, in one registry, over one injected
/// client: create, read, approve, merge.
///
/// Two deviations from the story's literal expectations, both forced by
/// behaviour earlier stories froze. `git.pr_create` consults neither the reviews
/// nor the check runs endpoint (US-025, `PrSignals::unknown`), so its
/// `review_state` is `none` and its `checks_state` is `unknown`: it reports what
/// it looked at rather than inventing an answer. And the merge pre-read of
/// FR-NEW-311 adds a `GET /pulls/42` before the merge, which is where the
/// second read of that route in the story's queue lands.
#[test]
fn e2e_new_796_create_read_review_merge_on_github() {
    with_hosts_lock(async {
        let e = PrEnv::new().await;
        let out = github_review_flow(&e).await;

        assert_eq!(out[0]["state"], "open");
        assert_eq!(out[0]["review_state"], "none", "pr_create consults no review endpoint");
        assert_eq!(out[0]["checks_state"], "unknown");
        assert_eq!(out[1]["state"], "open");
        assert_eq!(out[1]["review_state"], "review_required");
        assert_eq!(out[1]["checks_state"], "success");
        assert_eq!(out[2]["review_state"], "approved");
        assert_eq!(out[3]["state"], "merged");

        assert_eq!(
            e.api.steps(),
            vec![
                step("GET", "/repos/acme/api/branches/feature/pr-tools"),
                step("POST", "/repos/acme/api/pulls"),
                step("GET", GH_PR),
                step("GET", GH_REVIEWS),
                step("GET", &gh_check_runs()),
                step("POST", GH_REVIEW),
                step("GET", GH_PR),
                step("PUT", GH_MERGE),
            ]
        );
        assert!(e.api.calls().iter().all(|c| c.status != 599), "every call was routed");
        assert!(!serde_json::to_string(&out).unwrap().contains("ghp_"));
    });
}

/// E2E-NEW-797: the GitLab mirror. The point is not the routes, which differ by
/// design, but that the four results are shape identical to their GitHub
/// counterparts, key set and key order included, and that the states walk the
/// same path.
#[test]
fn e2e_new_797_the_gitlab_flow_returns_the_same_four_shapes() {
    with_hosts_lock(async {
        let gh_env = PrEnv::new().await;
        let gh = github_review_flow(&gh_env).await;
        let gl_env = PrEnv::new().await;
        let gl = gitlab_review_flow(&gl_env).await;

        for (i, (a, b)) in gh.iter().zip(gl.iter()).enumerate() {
            assert_eq!(keys_of(a), keys_of(b), "result {i} differs in key set or order");
        }
        assert_eq!(keys_of(&gh[2]), REVIEW_KEYS);

        assert_eq!(gl[0]["state"], "open");
        assert_eq!(gl[1]["state"], "open");
        assert_eq!(gl[2]["review_state"], "approved");
        assert_eq!(gl[3]["state"], "merged");

        assert_eq!(
            gl_env.api.steps(),
            vec![
                step("GET", "/projects/acme%2Fapi/repository/branches/feature/pr-tools"),
                step("POST", "/projects/acme%2Fapi/merge_requests"),
                step("GET", GL_MR),
                step("GET", GL_APPROVALS),
                step("POST", GL_APPROVE),
                step("GET", GL_APPROVALS),
                step("GET", GL_MR),
                step("PUT", GL_MERGE),
            ]
        );
        assert!(!serde_json::to_string(&gl).unwrap().contains("glpat"));
    });
}
