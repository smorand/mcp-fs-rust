//! Wire level tests of the git fetch negotiation (SPEC-0067).
//!
//! The contract of the negotiation is a reference: for a request, the service answers like git's
//! own server (`git upload-pack --stateless-rpc`) for the same project data, except for the
//! declared differences of the spec. So these tests do not hand enumerate protocol cases: they
//! send a request to the service over HTTP, run the same bytes through the reference program on
//! a bare copy of the same project, and compare the acknowledgement lines line by line and the
//! objects as a set. Real `git` clients cover the user journeys.
//!
//! Needs the `git` program and loopback networking; a missing `git` fails the test, it does not
//! skip it (same rule as `full_stack_e2e`). No external service: the server runs in process on
//! SQLite and a local blob directory.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use mcp_fs_core::app;
use mcp_fs_core::config::ServerConfig;
use mcp_fs_core::keys;
use serde_json::{Value, json};

const ADMIN: &str = "neg-admin@mcp-fs.test";
const STRANGER: &str = "neg-stranger@mcp-fs.test";
/// rmcp answers 406 to a `/mcp` POST whose `Accept` lacks both media types.
const MCP_ACCEPT: &str = "application/json, text/event-stream";
/// The options the service offers: the reference program runs with the same ones.
const CAPS: &str = "multi_ack_detailed side-band-64k ofs-delta";

// ── the in process server ────────────────────────────────────────────────────

/// The state every test of this binary shares: two servers over one state directory, one
/// refusing anonymous reads and one allowing them. The git repository store of the service is
/// process wide and fixed by the first server built (`GitRepoStore::shared`), so servers with
/// separate directories cannot coexist in one process; they run on a runtime of their own so
/// that no test's runtime ending takes them down. Projects are named per test.
struct Fixture {
    closed: String,
    open: String,
    key: PathBuf,
}

static FIXTURE: OnceLock<Fixture> = OnceLock::new();

fn fixture() -> &'static Fixture {
    FIXTURE.get_or_init(|| {
        require_git();
        let root = std::env::temp_dir().join(format!("mcpfs-neg-e2e-{}", std::process::id()));
        remove_stale_roots(&root);
        std::fs::create_dir_all(&root).expect("fixture dir");
        let (private_key, public_key) = keys::write_keypair(&root).expect("keypair");
        let (tx, rx) = std::sync::mpsc::channel();
        let key = private_key.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("fixture runtime");
            rt.block_on(async move {
                let mut bases = Vec::new();
                for anonymous_read in [false, true] {
                    let mut c = ServerConfig::default();
                    c.auth.jwt.public_key_path = public_key.display().to_string();
                    c.auth.admins = vec![ADMIN.to_string()];
                    c.infra.meta.dir = root.join("state/volumes").display().to_string();
                    c.infra.blob.dir = root.join("state/blobs").display().to_string();
                    c.infra.admin.path = root.join("state/admin.db").display().to_string();
                    c.git.enabled = true;
                    c.git.anonymous_read = anonymous_read;
                    let router = app::build(c).await.expect("app::build");
                    let listener =
                        tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
                    let port = listener.local_addr().expect("addr").port();
                    bases.push(format!("http://127.0.0.1:{port}"));
                    tokio::spawn(
                        async move { axum::serve(listener, router).await.expect("server") },
                    );
                }
                tx.send(bases).expect("send bases");
                std::future::pending::<()>().await;
            });
        });
        let bases = rx.recv().expect("fixture servers");
        Fixture { closed: bases[0].clone(), open: bases[1].clone(), key }
    })
}

/// Fixture directories left by earlier runs of this binary (a static is never dropped, so the
/// run itself cannot clean up).
fn remove_stale_roots(current: &Path) {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("mcpfs-neg-e2e-") && e.path() != current {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

struct Server {
    base: String,
    key: PathBuf,
    http: reqwest::Client,
}

impl Server {
    async fn start(anonymous_read: bool) -> Server {
        let f = fixture();
        Server {
            base: if anonymous_read { f.open.clone() } else { f.closed.clone() },
            key: f.key.clone(),
            http: reqwest::Client::new(),
        }
    }

    fn token(&self, who: &str) -> String {
        keys::mint_token_from_file(&self.key, who, keys::DEFAULT_ISSUER, keys::DEFAULT_CLAIM, 3600)
            .expect("token")
    }

    fn url(&self, project: &str) -> String {
        format!("{}/git/{project}", self.base)
    }

    /// One MCP `tools/call` as the admin, decoded from the server's SSE framing.
    async fn call_tool(&self, name: &str, arguments: Value) -> Value {
        let token = self.token(ADMIN);
        let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "git-negotiation-e2e", "version": "0.0"}}});
        let resp = self
            .http
            .post(format!("{}/mcp", self.base))
            .bearer_auth(&token)
            .header("Accept", MCP_ACCEPT)
            .json(&init)
            .send()
            .await
            .expect("initialize");
        assert!(resp.status().is_success(), "initialize: {}", resp.status());
        let session = resp
            .headers()
            .get("Mcp-Session-Id")
            .expect("session id")
            .to_str()
            .expect("ascii")
            .to_string();
        let ok = self
            .http
            .post(format!("{}/mcp", self.base))
            .bearer_auth(&token)
            .header("Accept", MCP_ACCEPT)
            .header("Mcp-Session-Id", &session)
            .json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .send()
            .await
            .expect("initialized");
        assert!(ok.status().is_success());
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}});
        let resp = self
            .http
            .post(format!("{}/mcp", self.base))
            .bearer_auth(&token)
            .header("Accept", MCP_ACCEPT)
            .header("Mcp-Session-Id", &session)
            .json(&body)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let status = resp.status();
        let text = resp.text().await.expect("body");
        assert!(status.is_success(), "{name}: http {status}: {text}");
        let envelope: Value = text
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|p| serde_json::from_str::<Value>(p.trim()).ok())
            .next_back()
            .unwrap_or_else(|| panic!("{name}: not JSON: {text}"));
        assert!(envelope.get("error").is_none(), "{name}: {envelope}");
        let content = envelope["result"]["content"][0]["text"].as_str().expect("content");
        serde_json::from_str(content)
            .unwrap_or_else(|e| panic!("{name}: the result is not JSON ({e}): {content}"))
    }

    /// A project the admin owns, with its git repository initialised and no commit.
    async fn project(&self, id: &str) {
        self.call_tool("admin.create_project", json!({"project_id": id, "owner": ADMIN})).await;
        self.call_tool("git.init", json!({"mount_id": id})).await;
    }

    /// POST a hand written request; returns the status and the body.
    async fn upload_pack(&self, project: &str, token: Option<&str>, body: &[u8]) -> (u16, Vec<u8>) {
        let mut rb = self
            .http
            .post(format!("{}/git-upload-pack", self.url(project)))
            .header("Content-Type", "application/x-git-upload-pack-request")
            .body(body.to_vec());
        if let Some(t) = token {
            rb = rb.bearer_auth(t);
        }
        let resp = rb.send().await.expect("upload-pack");
        let status = resp.status().as_u16();
        (status, resp.bytes().await.expect("body").to_vec())
    }
}

fn unique_project() -> String {
    format!("demo-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

fn require_git() {
    let ok =
        Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    assert!(ok, "the real `git` program is required on PATH for git_negotiation_e2e");
}

// ── the git program ──────────────────────────────────────────────────────────

fn git(dir: &Path, token: Option<&str>, args: &[&str]) -> Output {
    let mut c = Command::new("git");
    c.current_dir(dir);
    if let Some(t) = token {
        c.arg("-c").arg(format!("http.extraHeader=Authorization: Bearer {t}"));
    }
    c.args(args).output().unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

fn git_ok(dir: &Path, token: Option<&str>, args: &[&str]) -> String {
    let out = git(dir, token, args);
    assert!(
        out.status.success(),
        "git {args:?} in {dir:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A working repository on `main` with the identity git needs to commit.
fn work_repo(dir: &Path) {
    git_ok(dir, None, &["init", "-q", "-b", "main"]);
    git_ok(dir, None, &["config", "user.email", "neg@mcp-fs.test"]);
    git_ok(dir, None, &["config", "user.name", "neg"]);
}

/// Commit `file` with `content` on the current branch; returns the commit id.
fn commit_file(dir: &Path, file: &str, content: &str, message: &str) -> String {
    if let Some(parent) = Path::new(file).parent() {
        std::fs::create_dir_all(dir.join(parent)).expect("mkdir");
    }
    std::fs::write(dir.join(file), content).expect("write");
    git_ok(dir, None, &["add", "-A"]);
    git_ok(dir, None, &["commit", "-q", "-m", message]);
    git_ok(dir, None, &["rev-parse", "HEAD"])
}

/// Everything a baseline test needs: a server, a project holding `n` commits (c1 to cn,
/// `f<i>.txt` added by ci) pushed from a working repository.
struct Env {
    server: Server,
    /// Unique per test: the servers of this binary share one state directory.
    project: String,
    work: tempfile::TempDir,
    shas: Vec<String>,
}

impl Env {
    /// A project with no commit yet, and an empty working repository.
    async fn new(anonymous_read: bool) -> Env {
        let server = Server::start(anonymous_read).await;
        let project = unique_project();
        server.project(&project).await;
        let work = tempfile::tempdir().expect("work dir");
        work_repo(work.path());
        Env { server, project, work, shas: Vec::new() }
    }

    async fn with_history(n: usize, anonymous_read: bool) -> Env {
        let mut env = Env::new(anonymous_read).await;
        for i in 1..=n {
            let sha = commit_file(
                env.work.path(),
                &format!("f{i}.txt"),
                &format!("file {i}\n"),
                &format!("c{i}"),
            );
            env.shas.push(sha);
        }
        env.push();
        env
    }

    fn admin(&self) -> String {
        self.server.token(ADMIN)
    }

    /// Push the working repository's `main` to the project.
    fn push(&self) {
        let t = self.admin();
        git_ok(
            self.work.path(),
            Some(&t),
            &["push", "-q", &self.server.url(&self.project), "main:refs/heads/main"],
        );
    }

    /// A bare copy of the project's data: what the reference program serves.
    fn bare_copy(&self) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("bare dir");
        let t = self.admin();
        let target = dir.path().join("ref.git");
        git_ok(
            dir.path(),
            Some(&t),
            &["clone", "-q", "--bare", &self.server.url(&self.project), target.to_str().unwrap()],
        );
        dir
    }

    /// The project's branches and tags as `git ls-remote` shows them.
    fn refs(&self) -> String {
        let t = self.admin();
        git_ok(self.work.path(), Some(&t), &["ls-remote", &self.server.url(&self.project)])
    }
}

/// A working copy of the project made with a real `git clone`, ready to commit locally.
fn clone_copy(env: &Env, token: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("copy dir");
    let copy = dir.path().join("copy");
    git_ok(
        dir.path(),
        token,
        &["clone", "-q", &env.server.url(&env.project), copy.to_str().unwrap()],
    );
    git_ok(&copy, None, &["config", "user.email", "neg@mcp-fs.test"]);
    git_ok(&copy, None, &["config", "user.name", "neg"]);
    (dir, copy)
}

fn fetch(copy: &Path, token: Option<&str>) {
    git_ok(copy, token, &["fetch", "-q", "--tags"]);
}

/// `git fsck` on a copy: the integrity check of the spec.
fn assert_intact(copy: &Path) {
    git_ok(copy, None, &["fsck", "--strict"]);
}

fn count_commits(copy: &Path, rev: &str) -> usize {
    git_ok(copy, None, &["rev-list", "--count", rev]).parse().expect("count")
}

fn holds_any(answer: &Answer, ids: &[&str]) -> bool {
    ids.iter().any(|i| answer.objects.contains(*i))
}

// ── requests and answers ─────────────────────────────────────────────────────

fn pkt(line: &str) -> Vec<u8> {
    format!("{:04x}{line}", line.len() + 4).into_bytes()
}

/// The lines a git client sends: the wants (options on the first), a flush, the haves, then
/// the end marker or a flush.
fn request(wants: &[&str], haves: &[&str], done: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, w) in wants.iter().enumerate() {
        let caps = if i == 0 { format!(" {CAPS}") } else { String::new() };
        out.extend(pkt(&format!("want {w}{caps}\n")));
    }
    out.extend(b"0000");
    for h in haves {
        out.extend(pkt(&format!("have {h}\n")));
    }
    if done {
        out.extend(pkt("done\n"));
    } else {
        out.extend(b"0000");
    }
    out
}

/// What a client observes of an answer: the acknowledgement lines in order and the set of
/// objects received. The packing and the order of the objects are not compared.
#[derive(Debug, PartialEq)]
struct Answer {
    lines: Vec<String>,
    objects: BTreeSet<String>,
}

fn parse_answer(body: &[u8]) -> Answer {
    let (mut lines, mut pack) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < body.len() {
        assert!(i + 4 <= body.len(), "trailing fragment of {} bytes", body.len() - i);
        let len = usize::from_str_radix(std::str::from_utf8(&body[i..i + 4]).expect("hex"), 16)
            .expect("pkt length");
        if len == 0 {
            i += 4;
            continue;
        }
        assert!(len >= 4 && i + len <= body.len(), "bad pkt length {len} at {i}");
        let data = &body[i + 4..i + len];
        i += len;
        match data.first() {
            Some(1) => pack.extend_from_slice(&data[1..]),
            Some(2) | None => {}
            Some(3) => {
                panic!("fatal error on side-band 3: {}", String::from_utf8_lossy(&data[1..]))
            }
            Some(_) => {
                let line = String::from_utf8_lossy(data).trim_end().to_string();
                assert!(line.starts_with("ACK ") || line == "NAK", "unexpected line: {line:?}");
                lines.push(line);
            }
        }
    }
    Answer { lines, objects: pack_objects(&pack) }
}

/// The object ids a pack carries, listed by indexing it into a scratch repository.
fn pack_objects(pack: &[u8]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    if pack.is_empty() {
        return set;
    }
    assert!(pack.len() >= 32 && &pack[..4] == b"PACK", "not a pack: {} bytes", pack.len());
    // a pack header says how many objects follow; an empty one carries none
    if u32::from_be_bytes([pack[8], pack[9], pack[10], pack[11]]) == 0 {
        return set;
    }
    let dir = tempfile::tempdir().expect("scratch");
    let repo = git2::Repository::init_bare(dir.path()).expect("scratch repo");
    let odb = repo.odb().expect("odb");
    let mut writer = odb.packwriter().expect("packwriter");
    writer.write_all(pack).expect("write pack");
    writer.commit().expect("index pack");
    odb.foreach(|oid| {
        set.insert(oid.to_string());
        true
    })
    .expect("list objects");
    set
}

/// The reference answer: git's own server on a bare copy of the project data.
fn reference_answer(bare: &Path, request: &[u8]) -> Answer {
    let mut child = Command::new("git")
        .args(["upload-pack", "--stateless-rpc", bare.join("ref.git").to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn git upload-pack");
    let mut stdin = child.stdin.take().expect("stdin");
    let body = request.to_vec();
    // written from a thread: a large request must not deadlock against the output pipe
    let writer = std::thread::spawn(move || stdin.write_all(&body));
    let out = child.wait_with_output().expect("reference output");
    let _ = writer.join();
    assert!(
        out.status.success(),
        "the reference program failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    parse_answer(&out.stdout)
}

/// Our answer to the same request, over HTTP as the admin.
async fn our_answer(env: &Env, request: &[u8]) -> Answer {
    our_answer_as(env, request, Some(&env.admin())).await
}

async fn our_answer_as(env: &Env, request: &[u8], token: Option<&str>) -> Answer {
    let (status, body) = env.server.upload_pack(&env.project, token, request).await;
    assert_eq!(status, 200, "upload-pack status");
    parse_answer(&body)
}

fn assert_same_answer(ours: &Answer, reference: &Answer) {
    assert_eq!(ours.lines, reference.lines, "acknowledgement lines differ from the reference");
    assert_eq!(ours.objects, reference.objects, "object sets differ from the reference");
}

/// The audit log and the trash of the project: reads must leave both as they were.
async fn side_state(env: &Env) -> (Value, Value) {
    (
        env.server.call_tool("fs.audit_log", json!({"mount_id": env.project, "limit": 100})).await,
        env.server.call_tool("fs.trash_list", json!({"mount_id": env.project})).await,
    )
}

// ── baseline guards (behavior that must not change) ─────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn e2e_001_clone_of_a_four_commit_project() {
    let env = Env::with_history(4, false).await;
    let (refs0, side0) = (env.refs(), side_state(&env).await);
    let into = tempfile::tempdir().unwrap();
    let copy = into.path().join("copy");
    git_ok(
        into.path(),
        Some(&env.admin()),
        &["clone", "-q", &env.server.url(&env.project), copy.to_str().unwrap()],
    );
    assert_eq!(git_ok(&copy, None, &["rev-list", "--count", "HEAD"]), "4");
    assert_eq!(git_ok(&copy, None, &["rev-parse", "HEAD"]), env.shas[3]);
    git_ok(&copy, None, &["fsck", "--strict"]);

    // the same clone as a hand written request: same objects as the reference
    let bare = env.bare_copy();
    let req = request(&[&env.shas[3]], &[], true);
    let ours = our_answer(&env, &req).await;
    assert_eq!(ours.objects.len(), 12, "4 commits, 4 folder listings, 4 files");
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(env.refs(), refs0, "a clone changes nothing");
    assert_eq!(side_state(&env).await, side0, "audit log and trash are unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_012_wants_and_nothing_else() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let tip = &env.shas[5];
    // the reference sends nothing without an end marker; the service keeps sending everything
    let ours = our_answer(&env, &request(&[tip], &[], false)).await;
    let everything = reference_answer(bare.path(), &request(&[tip], &[], true));
    // a clone's closing line is a bare NAK for the reference too
    assert_eq!(ours.lines, everything.lines);
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert_eq!(ours.objects, everything.objects);
    assert_eq!(ours.objects.len(), 6 * 3, "6 commits, 6 folder listings, 6 files");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_013_empty_project() {
    let server = Server::start(false).await;
    let project = unique_project();
    server.project(&project).await;
    let token = server.token(ADMIN);
    let into = tempfile::tempdir().unwrap();
    let copy = into.path().join("copy");
    let out =
        git(into.path(), Some(&token), &["clone", &server.url(&project), copy.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("You appear to have cloned an empty repository."),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!git(&copy, None, &["rev-parse", "--verify", "HEAD"]).status.success(), "no commit");
    assert_eq!(
        git_ok(into.path(), Some(&token), &["ls-remote", &server.url(&project)]),
        "",
        "the project is unchanged: still no ref"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_015_wanted_object_unknown_to_the_project() {
    let env = Env::with_history(4, false).await;
    let before = env.refs();
    let unknown = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let ours = our_answer(&env, &request(&[unknown], &[], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_016_wanted_object_that_is_not_an_id() {
    let env = Env::with_history(4, false).await;
    let before = env.refs();
    let ours = our_answer(&env, &request(&["xyz"], &[], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_017_empty_request() {
    let env = Env::with_history(4, false).await;
    let before = env.refs();
    let (status, body) = env.server.upload_pack(&env.project, Some(&env.admin()), b"").await;
    assert_eq!(status, 200);
    assert_eq!(body, b"0000".to_vec(), "a request that wants nothing gets a bare flush");
    assert_eq!(env.refs(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_019_a_non_member_is_refused() {
    let env = Env::with_history(4, false).await;
    let stranger = env.server.token(STRANGER);
    let (status, body) = env
        .server
        .upload_pack(&env.project, Some(&stranger), &request(&[&env.shas[3]], &[], true))
        .await;
    assert_eq!(status, 403, "{}", String::from_utf8_lossy(&body));
    assert!(!body.windows(4).any(|w| w == b"PACK"), "no object is sent");
    let out = git(env.work.path(), Some(&stranger), &["ls-remote", &env.server.url(&env.project)]);
    assert!(!out.status.success(), "the git program is refused too");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_020_anonymous_caller_refused_when_anonymous_reading_is_off() {
    let env = Env::with_history(4, false).await;
    let (status, body) =
        env.server.upload_pack(&env.project, None, &request(&[&env.shas[3]], &[], true)).await;
    assert_eq!(status, 401, "{}", String::from_utf8_lossy(&body));
    assert!(!body.windows(4).any(|w| w == b"PACK"), "no object is sent");
    let out = git(env.work.path(), None, &["ls-remote", &env.server.url(&env.project)]);
    assert!(!out.status.success(), "the git program is refused too");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_022_reads_change_nothing() {
    let env = Env::with_history(4, false).await;
    let (refs0, side0) = (env.refs(), side_state(&env).await);
    let into = tempfile::tempdir().unwrap();
    let copy = into.path().join("copy");
    // a clone, then a fetch with nothing new: nothing changes
    git_ok(
        into.path(),
        Some(&env.admin()),
        &["clone", "-q", &env.server.url(&env.project), copy.to_str().unwrap()],
    );
    git_ok(&copy, Some(&env.admin()), &["fetch", "-q"]);
    assert_eq!(env.refs(), refs0, "branches and tags are identical");
    assert_eq!(side_state(&env).await, side0, "no audit entry, trash unchanged");
    // 2 new commits arrive by a push; the fetch that takes them changes nothing more
    let c5 = commit_file(env.work.path(), "g5.txt", "five\n", "c5");
    let c6 = commit_file(env.work.path(), "g6.txt", "six\n", "c6");
    env.push();
    let refs1 = env.refs();
    assert!(refs1.contains(&c6) && !refs1.contains(&c5), "only the new tip is listed: {refs1}");
    let side1 = side_state(&env).await;
    git_ok(&copy, Some(&env.admin()), &["fetch", "-q"]);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c6);
    assert_eq!(env.refs(), refs1, "branches and tags are identical");
    assert_eq!(side_state(&env).await, side1, "the fetch added no audit entry and left the trash");
}

// ── the end of the exchange (US-0136) ───────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn e2e_002_fetch_of_two_new_commits() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let c5 = commit_file(env.work.path(), "c.txt", "c\n", "c5");
    let c6 = commit_file(env.work.path(), "f1.txt", "changed\n", "c6");
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(count_commits(&copy, "origin/main"), 6);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c6);
    assert_intact(&copy);

    // what the client sent: wants c6, holds c4, ends the negotiation
    let bare = env.bare_copy();
    let req = request(&[&c6], &[&env.shas[3]], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects.len(), 6, "commit, folder listing and file, for c5 and for c6");
    let old: Vec<&str> = env.shas.iter().map(String::as_str).collect();
    assert!(!holds_any(&ours, &old) && ours.objects.contains(&c5));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_003_fetch_with_nothing_new() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let refs0 = env.refs();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(count_commits(&copy, "HEAD"), 4);
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let req = request(&[c4], &[c4], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.objects.is_empty(), "nothing to send");
    assert_eq!(ours.lines, vec![format!("ACK {c4} common"), format!("ACK {c4}")]);
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_004_the_copy_holds_commits_the_project_lacks() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let mut locals = Vec::new();
    for i in 1..=3 {
        locals.push(commit_file(&copy, &format!("l{i}.txt"), "local\n", &format!("l{i}")));
    }
    let c5 = commit_file(env.work.path(), "d.txt", "d\n", "c5");
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c5);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "HEAD"]), locals[2], "local commits are kept");
    assert_intact(&copy);

    let bare = env.bare_copy();
    let haves: Vec<&str> =
        [locals[2].as_str(), locals[1].as_str(), locals[0].as_str(), env.shas[3].as_str()].to_vec();
    let req = request(&[&c5], &haves, true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects.len(), 3, "the commit, its folder listing, d.txt");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_005_forty_local_commits_two_in_common() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    for i in 1..=36 {
        commit_file(&copy, &format!("local{i}.txt"), "local\n", &format!("local {i}"));
    }
    assert_eq!(count_commits(&copy, "HEAD"), 40);
    let c5 = commit_file(env.work.path(), "d.txt", "d\n", "c5");
    let c6 = commit_file(env.work.path(), "e.txt", "e\n", "c6");
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c6);
    assert_eq!(count_commits(&copy, "HEAD"), 40, "the 40 commits are kept");
    assert_intact(&copy);

    // the last exchange as the client sends it: every commit it holds, newest first
    let held = git_ok(&copy, None, &["rev-list", "HEAD"]);
    let haves: Vec<&str> = held.lines().collect();
    let bare = env.bare_copy();
    let req = request(&[&c6], &haves, true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects.len(), 6, "only c5 and c6");
    assert!(ours.objects.contains(&c5) && ours.objects.contains(&c6));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_006_merge_history_with_one_side_held() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let w = env.work.path();
    git_ok(w, None, &["checkout", "-q", "-b", "side", &env.shas[2]]);
    let s1 = commit_file(w, "side.txt", "side\n", "s1");
    git_ok(w, None, &["checkout", "-q", "main"]);
    git_ok(w, None, &["merge", "-q", "--no-ff", "-m", "m1", "side"]);
    let m1 = git_ok(w, None, &["rev-parse", "HEAD"]);
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), m1);
    assert_intact(&copy);

    let bare = env.bare_copy();
    let req = request(&[&m1], &[&env.shas[3]], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.objects.contains(&s1) && ours.objects.contains(&m1));
    let old: Vec<&str> = env.shas.iter().map(String::as_str).collect();
    assert!(!holds_any(&ours, &old), "no commit of c1 to c4 is sent");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_007_a_file_deleted_long_ago_and_added_back() {
    let env = Env::new(false).await;
    let w = env.work.path();
    commit_file(w, "old.txt", "old\n", "c1");
    git_ok(w, None, &["rm", "-q", "old.txt"]);
    git_ok(w, None, &["commit", "-q", "-m", "c2"]);
    commit_file(w, "other.txt", "other\n", "c3");
    let c3 = git_ok(w, None, &["rev-parse", "HEAD"]);
    env.push();
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let c4 = commit_file(w, "old.txt", "old\n", "c4");
    env.push();
    fetch(&copy, Some(&env.admin()));
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c4);
    assert_intact(&copy);

    // whichever objects git sends here, the service sends the same ones
    let bare = env.bare_copy();
    let req = request(&[&c4], &[&c3], true);
    assert_same_answer(&our_answer(&env, &req).await, &reference_answer(bare.path(), &req));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_011_twelve_unknown_held_commits() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let unknown: Vec<String> = (0..12).map(|i| format!("{:040x}", 0x1000 + i)).collect();
    let haves: Vec<&str> = unknown.iter().map(String::as_str).collect();
    let req = request(&[&env.shas[5]], &haves, true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.lines, vec!["NAK".to_string()], "no acknowledgement for an unknown id");
    assert_eq!(ours.objects.len(), 18, "everything reachable from c6");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_014_a_held_id_that_is_not_a_valid_id() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let tip = &env.shas[5];
    let ours = our_answer(&env, &request(&[tip], &["not-an-id"], true)).await;
    // as if the id had not been stated: the reference answer to the request without it
    let reference = reference_answer(bare.path(), &request(&[tip], &[], true));
    assert_same_answer(&ours, &reference);
    assert!(ours.lines.iter().all(|l| !l.contains("not-an-id")));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_021_anonymous_caller_allowed_when_anonymous_reading_is_on() {
    let env = Env::with_history(4, true).await;
    let (_d, copy) = clone_copy(&env, None);
    let c5 = commit_file(env.work.path(), "c.txt", "c\n", "c5");
    let c6 = commit_file(env.work.path(), "f1.txt", "changed\n", "c6");
    env.push();
    fetch(&copy, None);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c6);
    assert_eq!(count_commits(&copy, "origin/main"), 6);
    let bare = env.bare_copy();
    let req = request(&[&c6], &[&env.shas[3]], true);
    let ours = our_answer_as(&env, &req, None).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects.len(), 6, "exactly the objects of c5 and c6");
    assert!(ours.objects.contains(&c5));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_029_held_id_of_a_file_version() {
    let env = Env::with_history(4, false).await;
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let blob = git_ok(env.work.path(), None, &["rev-parse", &format!("{c4}:f1.txt")]);
    let ours = our_answer(&env, &request(&[c4], &[&blob], true)).await;
    let reference = reference_answer(bare.path(), &request(&[c4], &[], true));
    assert_eq!(
        ours.lines,
        vec!["NAK".to_string()],
        "the reference would acknowledge it, the service does not"
    );
    assert!(ours.lines.iter().all(|l| !l.contains(&blob)));
    assert_eq!(ours.objects, reference.objects, "every object reachable from c4");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_032_held_id_with_only_ten_characters() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let short = &env.shas[3][..10];
    let ours = our_answer(&env, &request(&[&env.shas[5]], &[short], true)).await;
    let reference = reference_answer(bare.path(), &request(&[&env.shas[5]], &[], true));
    assert_same_answer(&ours, &reference);
    assert!(ours.lines.iter().all(|l| !l.contains(short)));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_033_held_id_of_a_folder_listing() {
    let env = Env::with_history(4, false).await;
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let tree = git_ok(env.work.path(), None, &["rev-parse", &format!("{c4}^{{tree}}")]);
    let ours = our_answer(&env, &request(&[c4], &[&tree], true)).await;
    let reference = reference_answer(bare.path(), &request(&[c4], &[], true));
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert_eq!(ours.objects, reference.objects, "every object reachable from c4");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_034_held_id_of_an_annotated_tag() {
    let env = Env::with_history(4, false).await;
    let w = env.work.path();
    let c5 = commit_file(w, "d.txt", "d\n", "c5");
    env.push();
    // the reference copy is taken before the tag is pushed: a clone that has to carry an
    // annotated tag is the subject of a later story
    let bare = env.bare_copy();
    git_ok(w, None, &["tag", "-a", "-m", "first release", "v1", &env.shas[3]]);
    git_ok(w, Some(&env.admin()), &["push", "-q", &env.server.url(&env.project), "v1"]);
    let tag = git_ok(w, None, &["rev-parse", "v1"]);
    let ours = our_answer(&env, &request(&[&c5], &[&tag], true)).await;
    let reference = reference_answer(bare.path(), &request(&[&c5], &[], true));
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.lines.iter().all(|l| !l.contains(&tag)));
    assert_eq!(ours.objects, reference.objects, "every object reachable from c5");
}

// ── the rounds that do not end the negotiation (US-0137) ─────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn e2e_010_negotiation_stopped_before_the_end() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let req = request(&[&env.shas[5]], &[&env.shas[2], &env.shas[3]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.objects.is_empty(), "no object data before the end");
    // c6 reaches the held commits, so git tells the client it has enough; the service does too
    assert_eq!(
        ours.lines,
        vec![
            format!("ACK {} common", env.shas[2]),
            format!("ACK {} common", env.shas[3]),
            format!("ACK {} ready", env.shas[3]),
            "NAK".to_string(),
        ]
    );
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_041_unusable_held_ids_without_the_end_of_the_negotiation() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let unknown: Vec<String> = (0..12).map(|i| format!("{:040x}", 0x1000 + i)).collect();
    let haves: Vec<&str> = unknown.iter().map(String::as_str).collect();
    let req = request(&[&env.shas[5]], &haves, false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_042_held_id_of_a_file_version_without_the_end_of_the_negotiation() {
    let env = Env::with_history(6, false).await;
    let refs0 = env.refs();
    let c4 = &env.shas[3];
    let blob = git_ok(env.work.path(), None, &["rev-parse", &format!("{c4}:f1.txt")]);
    let ours = our_answer(&env, &request(&[&env.shas[5]], &[&blob], false)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()], "the reference would acknowledge the id");
    assert!(ours.lines.iter().all(|l| !l.contains(&blob)));
    assert!(ours.objects.is_empty(), "no object data before the end");
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_043_malformed_held_id_without_the_end_of_the_negotiation() {
    let env = Env::with_history(6, false).await;
    let ours = our_answer(&env, &request(&[&env.shas[5]], &["not-an-id"], false)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert!(ours.lines.iter().all(|l| !l.contains("not-an-id")));
}

/// Beyond the story's tests: the two cases where git withholds `ready` (the rule of DEC-004).
#[tokio::test(flavor = "multi_thread")]
async fn ready_is_withheld_like_git_does() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let unknown = format!("{:040x}", 0x1234);
    // an id the project never held, beside a held one
    let req = request(&[&env.shas[5]], &[&unknown, &env.shas[3]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.lines.iter().all(|l| !l.ends_with("ready")));
    // a want older than the held commit: it does not reach it
    let req = request(&[&env.shas[3]], &[&env.shas[5]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.lines.iter().all(|l| !l.ends_with("ready")));
    // the same held commit and a want that reaches it: ready
    let req = request(&[&env.shas[5]], &[&env.shas[3]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.lines.iter().any(|l| l.ends_with("ready")));
    // an unknown id met after a held commit that already satisfies the want: git answers
    // `ready` with that id, right where it meets it
    let req = request(&[&env.shas[5]], &[&env.shas[3], &unknown], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.lines[1], format!("ACK {unknown} ready"));
    // a want that is a folder listing leads to no commit: it does not hold `ready` back
    let tree = git_ok(env.work.path(), None, &["rev-parse", &format!("{}^{{tree}}", env.shas[5])]);
    let req = request(&[&tree], &[&env.shas[3]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert!(ours.lines.iter().any(|l| l.ends_with("ready")));
    // and the same exchange with the end marker keeps the `ready` of the unknown id
    let req = request(&[&env.shas[5]], &[&env.shas[3], &unknown], true);
    assert_same_answer(&our_answer(&env, &req).await, &reference_answer(bare.path(), &req));
}

// ── wanted ids that cannot be served, and annotated tags (US-0138) ───────────

/// A commit no branch or tag reaches: pushed under a branch name, then the branch is deleted.
/// The project keeps the object, which is what the service must not serve.
fn orphan_commit(env: &Env) -> String {
    let w = env.work.path();
    let t = env.admin();
    git_ok(w, None, &["checkout", "-q", "-b", "scratch"]);
    let o1 = commit_file(w, "orphan.txt", "orphan\n", "o1");
    git_ok(
        w,
        Some(&t),
        &["push", "-q", &env.server.url(&env.project), "scratch:refs/heads/scratch"],
    );
    git_ok(w, Some(&t), &["push", "-q", &env.server.url(&env.project), ":refs/heads/scratch"]);
    git_ok(w, None, &["checkout", "-q", "main"]);
    o1
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_008_annotated_tag_on_a_held_commit() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let w = env.work.path();
    git_ok(w, None, &["tag", "-a", "-m", "first release", "v1", &env.shas[3]]);
    git_ok(w, Some(&env.admin()), &["push", "-q", &env.server.url(&env.project), "v1"]);
    fetch(&copy, Some(&env.admin()));
    let tag = git_ok(&copy, None, &["rev-parse", "v1"]);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "v1^{commit}"]), env.shas[3]);
    assert!(git_ok(&copy, None, &["cat-file", "-p", "v1"]).ends_with("first release"));
    assert_intact(&copy);

    let bare = env.bare_copy();
    let req = request(&[&tag], &[&env.shas[3]], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects, BTreeSet::from([tag]), "the tag only");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_009_annotated_tag_on_a_new_commit() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let w = env.work.path();
    let c5 = commit_file(w, "d.txt", "d\n", "c5");
    git_ok(w, None, &["tag", "-a", "-m", "second release", "v2", &c5]);
    env.push();
    git_ok(w, Some(&env.admin()), &["push", "-q", &env.server.url(&env.project), "v2"]);
    fetch(&copy, Some(&env.admin()));
    let tag = git_ok(&copy, None, &["rev-parse", "v2"]);
    assert_eq!(git_ok(&copy, None, &["rev-parse", "v2^{commit}"]), c5);
    assert_intact(&copy);

    let bare = env.bare_copy();
    let req = request(&[&tag, &c5], &[&env.shas[3]], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects.len(), 4, "the tag, c5, its folder listing, d.txt");
    assert!(ours.objects.contains(&tag) && ours.objects.contains(&c5));
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_030_wanted_commit_that_no_branch_reaches() {
    let env = Env::with_history(4, false).await;
    let o1 = orphan_commit(&env);
    let refs0 = env.refs();
    assert!(!refs0.contains(&o1), "no ref reaches the orphan");
    let ours = our_answer(&env, &request(&[&o1], &[], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty(), "nothing is sent for it");
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_047_unreachable_wanted_commit_with_a_held_id_and_no_end() {
    let env = Env::with_history(4, false).await;
    let o1 = orphan_commit(&env);
    let refs0 = env.refs();
    let ours = our_answer(&env, &request(&[&o1], &[&env.shas[2]], false)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()], "no ACK line for the held id");
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_048_malformed_wanted_id_with_a_held_id() {
    let env = Env::with_history(4, false).await;
    let refs0 = env.refs();
    let ours = our_answer(&env, &request(&["xyz"], &[&env.shas[2]], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()], "no ACK line for the held id");
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_049_one_servable_and_one_unservable_wanted_id() {
    let env = Env::with_history(6, false).await;
    let o1 = orphan_commit(&env);
    let bare = env.bare_copy();
    let ours = our_answer(&env, &request(&[&env.shas[5], &o1], &[&env.shas[3]], true)).await;
    // git would refuse the orphan; the service answers on the servable want alone
    let reference = reference_answer(bare.path(), &request(&[&env.shas[5]], &[&env.shas[3]], true));
    assert_same_answer(&ours, &reference);
    assert!(!ours.objects.contains(&o1), "nothing of the orphan is sent");
}

/// Beyond the story's tests: a file version or a folder listing is wanted directly (FR-NEW-008
/// covers every kind of object): served when a branch reaches it, silent when none does.
#[tokio::test(flavor = "multi_thread")]
async fn a_wanted_file_version_is_served_only_when_a_ref_reaches_it() {
    let env = Env::with_history(4, false).await;
    let reachable =
        git_ok(env.work.path(), None, &["rev-parse", &format!("{}:f2.txt", env.shas[3])]);
    let o1 = orphan_commit(&env);
    let orphan_blob = git_ok(env.work.path(), None, &["rev-parse", &format!("{o1}:orphan.txt")]);
    let bare = env.bare_copy();

    let req = request(&[&reachable], &[], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.objects, BTreeSet::from([reachable]));

    let ours = our_answer(&env, &request(&[&orphan_blob], &[], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty(), "no branch reaches that file version");
}

// ── odd requests and options the service does not offer (US-0139) ───────────

/// A request with extra lines between the wanted lines and their flush, as a client with
/// options sends them: `extra` lines go right after the wants.
fn request_with(extra: &[&str], wants: &[&str], haves: &[&str], done: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, w) in wants.iter().enumerate() {
        let caps = if i == 0 { format!(" {CAPS}") } else { String::new() };
        out.extend(pkt(&format!("want {w}{caps}\n")));
    }
    for line in extra {
        out.extend(pkt(&format!("{line}\n")));
    }
    out.extend(b"0000");
    for h in haves {
        out.extend(pkt(&format!("have {h}\n")));
    }
    if done {
        out.extend(pkt("done\n"));
    } else {
        out.extend(b"0000");
    }
    out
}

/// A request whose first want carries one more option than the service offers.
fn request_with_option(option: &str, wants: &[&str], haves: &[&str], done: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, w) in wants.iter().enumerate() {
        let caps = if i == 0 { format!(" {CAPS} {option}") } else { String::new() };
        out.extend(pkt(&format!("want {w}{caps}\n")));
    }
    out.extend(b"0000");
    for h in haves {
        out.extend(pkt(&format!("have {h}\n")));
    }
    if done {
        out.extend(pkt("done\n"));
    } else {
        out.extend(b"0000");
    }
    out
}

/// Every object reachable from `tip`, as the reference sends it for a clone.
fn everything(bare: &Path, tip: &str) -> Answer {
    reference_answer(bare, &request(&[tip], &[], true))
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_031_deepen_option_with_a_held_id() {
    let env = Env::with_history(4, false).await;
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let refs0 = env.refs();
    let ours = our_answer(&env, &request_with(&["deepen 1"], &[c4], &[&env.shas[2]], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()], "no ACK line, no shallow line");
    assert_eq!(ours.objects, everything(bare.path(), c4).objects);
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_035_filter_option() {
    let env = Env::with_history(4, false).await;
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let ours =
        our_answer(&env, &request_with(&["filter blob:none"], &[c4], &[&env.shas[2]], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()], "no ACK line");
    assert_eq!(ours.objects, everything(bare.path(), c4).objects);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_036_shallow_line_from_the_client() {
    let env = Env::with_history(4, false).await;
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let line = format!("shallow {}", env.shas[2]);
    let ours = our_answer(&env, &request_with(&[&line], &[c4], &[], true)).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()], "no shallow line, no ACK line");
    assert_eq!(ours.objects, everything(bare.path(), c4).objects);
    // with a held id beside it, the option still wins over the negotiation
    let both = our_answer(&env, &request_with(&[&line], &[c4], &[&env.shas[1]], true)).await;
    assert_eq!(both, ours);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_037_held_ids_but_nothing_wanted() {
    let env = Env::with_history(4, false).await;
    let refs0 = env.refs();
    let (status, body) = env
        .server
        .upload_pack(&env.project, Some(&env.admin()), &request(&[], &[&env.shas[3]], true))
        .await;
    assert_eq!(status, 200);
    assert_eq!(body, b"0000".to_vec(), "the answer to a request that wants nothing");
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_044_no_done_option_that_the_service_does_not_offer() {
    let env = Env::with_history(4, false).await;
    let refs0 = env.refs();
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let with = request_with_option("no-done", &[c4], &[&env.shas[2]], false);
    let without = request(&[c4], &[&env.shas[2]], false);
    let ours = our_answer(&env, &with).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &without));
    assert!(ours.objects.is_empty(), "no object data");
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_045_include_tag_option_that_the_service_does_not_offer() {
    let env = Env::with_history(4, false).await;
    let w = env.work.path();
    git_ok(w, None, &["tag", "-a", "-m", "third", "v1", &env.shas[2]]);
    git_ok(w, Some(&env.admin()), &["push", "-q", &env.server.url(&env.project), "v1"]);
    let tag = git_ok(w, None, &["rev-parse", "v1"]);
    let refs0 = env.refs();
    let bare = env.bare_copy();
    let c4 = &env.shas[3];
    let with = our_answer(&env, &request_with_option("include-tag", &[c4], &[], true)).await;
    let without = our_answer(&env, &request(&[c4], &[], true)).await;
    assert_eq!(with, without, "the option changes nothing");
    assert_eq!(with.objects, everything(bare.path(), c4).objects);
    assert!(!with.objects.contains(&tag), "the tag object is not among them");
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_046_an_unknown_option() {
    let env = Env::with_history(4, false).await;
    let refs0 = env.refs();
    let c4 = &env.shas[3];
    let with = our_answer(&env, &request_with_option("x-foo", &[c4], &[&env.shas[2]], true)).await;
    let without = our_answer(&env, &request(&[c4], &[&env.shas[2]], true)).await;
    assert_eq!(with, without);
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_050_deepen_line_with_no_wanted_line() {
    let env = Env::with_history(4, false).await;
    let refs0 = env.refs();
    let req = request_with(&["deepen 1"], &[], &[&env.shas[2]], true);
    let (status, body) = env.server.upload_pack(&env.project, Some(&env.admin()), &req).await;
    assert_eq!(status, 200);
    assert_eq!(body, b"0000".to_vec(), "a bare flush, no NAK");
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_018_request_cut_in_the_middle_of_a_held_line() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let c6 = &env.shas[5];
    let refs0 = env.refs();
    // wants c6, holds c4, then a second held line whose last byte (the newline) is missing:
    // an id that looks complete but was cut, so it must not be read
    let complete = request(&[c6], &[&env.shas[3], &env.shas[2]], false);
    let cut = &complete[..complete.len() - 4 - 1]; // drop the closing flush and the newline
    let before_cut = request(&[c6], &[&env.shas[3]], false);
    let ours = our_answer(&env, cut).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &before_cut));
    assert!(ours.lines.iter().all(|l| !l.contains(&env.shas[2])), "the cut id is not read");
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_051_no_flush_after_the_wanted_line() {
    let env = Env::with_history(6, false).await;
    let refs0 = env.refs();
    let bare = env.bare_copy();
    let c6 = &env.shas[5];
    let mut req = pkt(&format!("want {c6} {CAPS}\n"));
    req.extend(pkt(&format!("have {}\n", env.shas[3])));
    let completed = request(&[c6], &[&env.shas[3]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &completed));
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_052_end_marker_without_a_flush_after_the_wanted_line() {
    let env = Env::with_history(6, false).await;
    let refs0 = env.refs();
    let bare = env.bare_copy();
    let c6 = &env.shas[5];
    let mut req = pkt(&format!("want {c6} {CAPS}\n"));
    req.extend(pkt(&format!("have {}\n", env.shas[3])));
    req.extend(pkt("done\n"));
    let completed = request(&[c6], &[&env.shas[3]], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &completed));
    assert_eq!(ours.objects.len(), 6, "every object of c5 and c6");
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_053_wanted_line_only_without_a_flush() {
    let env = Env::with_history(6, false).await;
    let refs0 = env.refs();
    let bare = env.bare_copy();
    let c6 = &env.shas[5];
    let req = pkt(&format!("want {c6} {CAPS}\n"));
    let ours = our_answer(&env, &req).await;
    // a clone: the reference sends every object once the negotiation is ended
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert_eq!(ours.objects, everything(bare.path(), c6).objects);
    assert_eq!(env.refs(), refs0, "the project is unchanged");
}

/// The other cuts: in the middle of a wanted line, of the end marker, and of a line header.
#[tokio::test(flavor = "multi_thread")]
async fn a_request_cut_elsewhere_is_read_up_to_the_cut() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let (c6, c4) = (&env.shas[5], &env.shas[3]);
    let refs0 = env.refs();
    // the only wanted line cut: nothing is wanted, the answer to a request that wants nothing
    let full = request(&[c6], &[c4], true);
    let (status, body) =
        env.server.upload_pack(&env.project, Some(&env.admin()), &full[..30]).await;
    assert_eq!((status, body), (200, b"0000".to_vec()));
    // the end marker cut after "do": the lines before it are answered, the negotiation is not ended
    let mut cut = request(&[c6], &[c4], false);
    cut.truncate(cut.len() - 4);
    cut.extend(b"0009do");
    let ours = our_answer(&env, &cut).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &request(&[c6], &[c4], false)));
    // a line header cut in two: the request ends there
    let mut stub = request(&[c6], &[c4], false);
    stub.truncate(stub.len() - 4);
    stub.extend(b"00");
    let ours = our_answer(&env, &stub).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &request(&[c6], &[c4], false)));
    assert_eq!(env.refs(), refs0);
}

// ── fetches during pushes (US-0140) ──────────────────────────────────────────

/// Incompressible text of about `kb` kilobytes: a push of several of these takes long enough
/// for fetches to overlap it.
fn heavy_text(seed: u64, kb: usize) -> String {
    let mut x = seed | 1;
    let mut out = String::with_capacity(kb * 1024);
    while out.len() < kb * 1024 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push_str(&format!("{x:016x}"));
    }
    out
}

/// What the fetches of a copy saw: the branch tips after each fetch, and how many fetches began
/// before the push was over (the ones that overlapped it).
struct Seen {
    tips: Vec<Vec<String>>,
    overlapping: usize,
}

/// Fetch into `copy` at least `min_rounds` times and until `pushed` is set; every fetch must
/// succeed and leave the copy intact. Records `origin/<branch>` after each fetch.
fn fetch_rounds(
    copy: &Path,
    token: &str,
    branches: &[&str],
    min_rounds: usize,
    pushed: &std::sync::atomic::AtomicBool,
) -> Seen {
    use std::sync::atomic::Ordering::SeqCst;
    let mut seen = Seen { tips: Vec::new(), overlapping: 0 };
    loop {
        if !pushed.load(SeqCst) {
            seen.overlapping += 1;
        }
        let out = git(copy, Some(token), &["fetch", "-q"]);
        assert!(
            out.status.success(),
            "a fetch during the push failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        git_ok(copy, None, &["fsck", "--strict"]);
        seen.tips.push(
            branches
                .iter()
                .map(|b| git_ok(copy, None, &["rev-parse", &format!("origin/{b}")]))
                .collect(),
        );
        if seen.tips.len() >= min_rounds && pushed.load(SeqCst) {
            return seen;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_025_fetch_during_a_push() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let c4 = env.shas[3].clone();
    let mut c8 = String::new();
    for i in 5..=8 {
        c8 = commit_file(
            env.work.path(),
            &format!("p{i}.txt"),
            &heavy_text(i as u64, 600),
            &format!("c{i}"),
        );
    }
    let (work, url, token) =
        (env.work.path().to_path_buf(), env.server.url(&env.project), env.admin());
    let pushed = Arc::new(AtomicBool::new(false));
    let pusher = {
        let (work, url, token, pushed) = (work.clone(), url.clone(), token.clone(), pushed.clone());
        tokio::task::spawn_blocking(move || {
            git_ok(&work, Some(&token), &["push", "-q", &url, "main:refs/heads/main"]);
            pushed.store(true, std::sync::atomic::Ordering::SeqCst);
        })
    };
    let (copy2, token2, flag) = (copy.clone(), token.clone(), pushed.clone());
    let fetcher =
        tokio::task::spawn_blocking(move || fetch_rounds(&copy2, &token2, &["main"], 3, &flag));
    let seen = fetcher.await.expect("fetches");
    pusher.await.expect("push");
    assert!(seen.overlapping >= 1, "no fetch began before the push was over");
    for tips in &seen.tips {
        assert!(tips[0] == c4 || tips[0] == c8, "a fetch saw {} between c4 and c8", tips[0]);
    }
    let last = fetch_rounds(&copy, &token, &["main"], 1, &pushed);
    assert_eq!(last.tips[0][0], c8, "once the push is over the copy gets all of it");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_026_a_push_that_fails_during_a_fetch() {
    let env = Env::with_history(4, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    let c4 = env.shas[3].clone();
    let c5 = commit_file(env.work.path(), "p5.txt", &heavy_text(5, 600), "c5");
    let stranger = env.server.token(STRANGER);
    let (work, url) = (env.work.path().to_path_buf(), env.server.url(&env.project));
    let pushed = Arc::new(AtomicBool::new(false));
    let pusher = {
        let (stranger, url, pushed) = (stranger, url, pushed.clone());
        tokio::task::spawn_blocking(move || {
            // not a member: the membership check refuses the request before any pack is read
            let out = git(&work, Some(&stranger), &["push", "-q", &url, "main:refs/heads/main"]);
            pushed.store(true, std::sync::atomic::Ordering::SeqCst);
            assert!(!out.status.success(), "the push is rejected");
        })
    };
    let (copy2, token, flag) = (copy.clone(), env.admin(), pushed.clone());
    let fetcher =
        tokio::task::spawn_blocking(move || fetch_rounds(&copy2, &token, &["main"], 3, &flag));
    let seen = fetcher.await.expect("fetches");
    pusher.await.expect("push");
    assert!(seen.tips.iter().all(|t| t[0] == c4), "the copy stays at c4");
    assert_eq!(git_ok(&copy, None, &["rev-parse", "HEAD"]), c4);
    let refs = env.refs();
    assert!(refs.contains(&format!("{c4}\trefs/heads/main")), "main is still at c4: {refs}");
    assert!(!refs.contains(&c5), "c5 never reached the project");
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_027_two_fetches_during_a_push_moving_two_branches() {
    let env = Env::with_history(4, false).await;
    let w = env.work.path();
    let c4 = env.shas[3].clone();
    git_ok(w, None, &["checkout", "-q", "-b", "feature"]);
    let f1 = commit_file(w, "feature1.txt", "f1\n", "f1");
    git_ok(
        w,
        Some(&env.admin()),
        &["push", "-q", &env.server.url(&env.project), "feature:refs/heads/feature"],
    );
    git_ok(w, None, &["checkout", "-q", "main"]);
    let (_d1, copy1) = clone_copy(&env, Some(&env.admin()));
    let (_d2, copy2) = clone_copy(&env, Some(&env.admin()));
    let mut c8 = String::new();
    for i in 5..=8 {
        c8 = commit_file(w, &format!("p{i}.txt"), &heavy_text(i as u64, 600), &format!("c{i}"));
    }
    git_ok(w, None, &["checkout", "-q", "feature"]);
    commit_file(w, "feature2.txt", &heavy_text(20, 300), "f2");
    let f3 = commit_file(w, "feature3.txt", &heavy_text(21, 300), "f3");
    let (work, url, token) = (w.to_path_buf(), env.server.url(&env.project), env.admin());
    let pushed = Arc::new(AtomicBool::new(false));
    let pusher = {
        let (url, token, pushed) = (url.clone(), token.clone(), pushed.clone());
        tokio::task::spawn_blocking(move || {
            git_ok(
                &work,
                Some(&token),
                &["push", "-q", &url, "main:refs/heads/main", "feature:refs/heads/feature"],
            );
            pushed.store(true, std::sync::atomic::Ordering::SeqCst);
        })
    };
    let fetchers: Vec<_> = [copy1.clone(), copy2.clone()]
        .into_iter()
        .map(|copy| {
            let (token, flag) = (token.clone(), pushed.clone());
            tokio::task::spawn_blocking(move || {
                fetch_rounds(&copy, &token, &["main", "feature"], 3, &flag)
            })
        })
        .collect();
    for f in fetchers {
        let seen = f.await.expect("fetches");
        assert!(seen.overlapping >= 1, "no fetch began before the push was over");
        for tips in &seen.tips {
            assert!(tips[0] == c4 || tips[0] == c8, "main saw {}", tips[0]);
            assert!(tips[1] == f1 || tips[1] == f3, "feature saw {}", tips[1]);
        }
    }
    pusher.await.expect("push");
    // the push is over: each copy gets both branches, whether or not they moved together
    for copy in [&copy1, &copy2] {
        let last = fetch_rounds(copy, &token, &["main", "feature"], 1, &pushed);
        assert_eq!(last.tips[0], vec![c8.clone(), f3.clone()]);
    }
}

/// A pack longer than one side band packet: the framing must keep every packet within the 65520
/// bytes the protocol allows, or the client stops with "bad line length". Found by E2E-025.
#[tokio::test(flavor = "multi_thread")]
async fn a_clone_larger_than_one_side_band_packet() {
    let env = Env::new(false).await;
    commit_file(env.work.path(), "big.txt", &heavy_text(7, 400), "big");
    env.push();
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    assert_eq!(count_commits(&copy, "HEAD"), 1);
    assert_intact(&copy);
    let req = request(&[&git_ok(&copy, None, &["rev-parse", "HEAD"])], &[], true);
    let (status, body) = env.server.upload_pack(&env.project, Some(&env.admin()), &req).await;
    assert_eq!(status, 200);
    let mut i = 0;
    while i + 4 <= body.len() {
        let len = usize::from_str_radix(std::str::from_utf8(&body[i..i + 4]).unwrap(), 16).unwrap();
        assert!(len <= 65520, "a packet of {len} bytes at {i}");
        i += len.max(4);
    }
}

// ── lines in an unexpected order (US-0142) ───────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn fr_new_014_a_held_id_before_the_wanted_line() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let (c6, c4) = (&env.shas[5], &env.shas[3]);
    let mut odd = pkt(&format!("have {c4}\n"));
    odd.extend(pkt(&format!("want {c6} {CAPS}\n")));
    odd.extend(b"0000");
    odd.extend(pkt("done\n"));
    let ours = our_answer(&env, &odd).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &request(&[c6], &[c4], true)));
    assert_eq!(ours.objects.len(), 6, "the objects of c5 and c6");
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn fr_new_014_a_deepen_line_after_the_held_ids() {
    let env = Env::with_history(4, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let (c4, c2) = (&env.shas[3], &env.shas[1]);
    let mut odd = pkt(&format!("want {c4} {CAPS}\n"));
    odd.extend(b"0000");
    odd.extend(pkt(&format!("have {c2}\n")));
    odd.extend(b"0000");
    odd.extend(pkt("deepen 1\n"));
    odd.extend(pkt("done\n"));
    let ours = our_answer(&env, &odd).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()], "the option still wins over the held id");
    assert_eq!(ours.objects, everything(bare.path(), c4).objects);
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn fr_new_014_lines_after_the_end_marker_and_control_packets_between_lines() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let (c6, c4) = (&env.shas[5], &env.shas[3]);
    let usual = request(&[c6], &[c4], true);
    let expected = reference_answer(bare.path(), &usual);
    // the wanted line and a held id after the end marker
    let mut odd = pkt("done\n");
    odd.extend(pkt(&format!("have {c4}\n")));
    odd.extend(pkt(&format!("want {c6} {CAPS}\n")));
    assert_same_answer(&our_answer(&env, &odd).await, &expected);
    // delimiter and response end packets between the lines are skipped like flushes
    let mut odd = pkt(&format!("want {c6} {CAPS}\n"));
    odd.extend(b"0001");
    odd.extend(pkt(&format!("have {c4}\n")));
    odd.extend(b"0002");
    odd.extend(pkt("done\n"));
    assert_same_answer(&our_answer(&env, &odd).await, &expected);
    // a deepen line after the end marker still counts
    let mut odd = request(&[c4], &[&env.shas[1]], true);
    odd.extend(pkt("deepen 1\n"));
    let ours = our_answer(&env, &odd).await;
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
}

// ── unknown held ids and the ready line (SPEC-0068) ──────────────────────────

/// A well formed id the project never held.
fn unknown_id(n: u32) -> String {
    format!("{:040x}", 0x1111_1111_u32.wrapping_mul(n))
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0068_e2e_001_unknown_ids_after_a_satisfying_held_commit() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let (c4, u1, u2) = (&env.shas[3], unknown_id(1), unknown_id(2));
    let req = request(&[&env.shas[5]], &[c4, &u1, &u2], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(
        ours.lines,
        vec![
            format!("ACK {c4} common"),
            format!("ACK {u1} ready"),
            format!("ACK {u2} ready"),
            "NAK".to_string()
        ]
    );
    assert!(ours.objects.is_empty());
    assert!(
        ours.lines.iter().all(|l| !(l.contains(&u1) || l.contains(&u2)) || l.ends_with("ready"))
    );
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0068_e2e_002_an_unknown_id_before_any_held_commit() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let (c4, u1, u2) = (&env.shas[3], unknown_id(1), unknown_id(2));
    let req = request(&[&env.shas[5]], &[&u1, c4, &u2], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(
        ours.lines,
        vec![format!("ACK {c4} common"), format!("ACK {u2} ready"), format!("ACK {c4}")]
    );
    assert_eq!(ours.objects.len(), 6, "the objects of c5 and c6");
    assert!(ours.lines.iter().all(|l| !l.contains(&u1)), "no line names U1");
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0068_e2e_003_the_held_commit_is_newer_than_the_wanted_one() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let (c6, u1) = (&env.shas[5], unknown_id(1));
    let req = request(&[&env.shas[3]], &[c6, &u1], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.lines, vec![format!("ACK {c6} common"), "NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0068_e2e_004_only_unknown_ids() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let req = request(&[&env.shas[5]], &[&unknown_id(1), &unknown_id(2)], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(ours.lines, vec!["NAK".to_string()]);
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0068_e2e_005_an_unknown_id_between_two_held_commits() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let (c3, c4, u1) = (&env.shas[2], &env.shas[3], unknown_id(1));
    let req = request(&[&env.shas[5]], &[c3, &u1, c4], true);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    assert_eq!(
        ours.lines,
        vec![
            format!("ACK {c3} common"),
            format!("ACK {u1} ready"),
            format!("ACK {c4} common"),
            format!("ACK {c4}"),
        ]
    );
    assert_eq!(ours.objects.len(), 6, "the objects of c5 and c6");
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0068_e2e_006_three_unknown_ids_after_a_satisfying_held_commit() {
    let env = Env::with_history(6, false).await;
    let bare = env.bare_copy();
    let refs0 = env.refs();
    let c4 = &env.shas[3];
    let unknown: Vec<String> = (1..=3).map(unknown_id).collect();
    let req = request(&[&env.shas[5]], &[c4, &unknown[0], &unknown[1], &unknown[2]], false);
    let ours = our_answer(&env, &req).await;
    assert_same_answer(&ours, &reference_answer(bare.path(), &req));
    let mut expected = vec![format!("ACK {c4} common")];
    expected.extend(unknown.iter().map(|u| format!("ACK {u} ready")));
    expected.push("NAK".to_string());
    assert_eq!(ours.lines, expected, "one ready line per unknown id, in the order stated");
    assert!(ours.objects.is_empty());
    assert_eq!(env.refs(), refs0);
}

// ── the options the service offers and honours (SPEC-0069) ───────────────────

/// A request whose first wanted line carries exactly `caps` as its options.
fn request_caps(caps: &str, wants: &[&str], haves: &[&str], done: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, w) in wants.iter().enumerate() {
        let c = if i == 0 { format!(" {caps}") } else { String::new() };
        out.extend(pkt(&format!("want {w}{c}\n")));
    }
    out.extend(b"0000");
    for h in haves {
        out.extend(pkt(&format!("have {h}\n")));
    }
    if done {
        out.extend(pkt("done\n"));
    } else {
        out.extend(b"0000");
    }
    out
}

/// The options announced with the first ref of the project's listing: the text between the NUL
/// byte and the newline of the first ref line.
async fn announced_options(env: &Env) -> String {
    let url = format!("{}/info/refs?service=git-upload-pack", env.server.url(&env.project));
    let body = env
        .server
        .http
        .get(url)
        .bearer_auth(env.admin())
        .send()
        .await
        .expect("info/refs")
        .bytes()
        .await
        .expect("body");
    let nul = body.iter().position(|b| *b == 0).expect("a NUL byte after the first ref");
    let end = body[nul..].iter().position(|b| *b == b'\n').expect("a newline") + nul;
    String::from_utf8(body[nul + 1..end].to_vec()).expect("utf8 options")
}

const OFFERED: &str = "multi_ack_detailed side-band-64k ofs-delta agent=mcp-fs/0.1.0";

#[tokio::test(flavor = "multi_thread")]
async fn spec_0069_e2e_001_a_real_client_still_clones_and_fetches() {
    let env = Env::with_history(6, false).await;
    let (_d, copy) = clone_copy(&env, Some(&env.admin()));
    assert_eq!(count_commits(&copy, "HEAD"), 6);
    let c7 = commit_file(env.work.path(), "g7.txt", "seven\n", "c7");
    env.push();
    let refs0 = env.refs();
    // the real client also asks for options the service does not offer (thin-pack, no-done...)
    fetch(&copy, Some(&env.admin()));
    assert_eq!(git_ok(&copy, None, &["rev-parse", "origin/main"]), c7);
    assert_eq!(count_commits(&copy, "origin/main"), 7);
    assert_intact(&copy);
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0069_e2e_002_the_options_the_service_offers() {
    let env = Env::with_history(6, false).await;
    let refs0 = env.refs();
    let options = announced_options(&env).await;
    assert_eq!(options, OFFERED);
    assert!(!options.split(' ').any(|o| o == "multi_ack"), "plain multi_ack is not offered");
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0069_e2e_004_a_client_that_asks_only_for_the_plain_multi_ack() {
    let env = Env::with_history(6, false).await;
    let refs0 = env.refs();
    let (c4, c6) = (&env.shas[3], &env.shas[5]);
    let plain =
        our_answer(&env, &request_caps("multi_ack side-band-64k ofs-delta", &[c6], &[c4], true))
            .await;
    let detailed = our_answer(&env, &request_caps(CAPS, &[c6], &[c4], true)).await;
    assert_eq!(plain, detailed, "the plain form is answered as the detailed one");
    assert_eq!(plain.lines, vec![format!("ACK {c4} common"), format!("ACK {c4}")]);
    assert_eq!(plain.objects.len(), 6);
    assert!(plain.lines.iter().all(|l| !l.contains("continue")));
    assert_eq!(env.refs(), refs0);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0069_e2e_006_the_options_of_an_empty_project() {
    let env = Env::new(false).await;
    assert_eq!(announced_options(&env).await, OFFERED);
    assert_eq!(env.refs(), "", "no branch is listed");
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_0069_e2e_007_several_options_the_service_does_not_offer() {
    let env = Env::with_history(6, false).await;
    let refs0 = env.refs();
    let (c4, c6) = (&env.shas[3], &env.shas[5]);
    let caps = "multi_ack multi_ack_detailed side-band-64k thin-pack include-tag";
    let many = our_answer(&env, &request_caps(caps, &[c6], &[c4], true)).await;
    let plain =
        our_answer(&env, &request_caps("multi_ack_detailed side-band-64k", &[c6], &[c4], true))
            .await;
    assert_eq!(many, plain);
    assert_eq!(many.lines, vec![format!("ACK {c4} common"), format!("ACK {c4}")]);
    assert_eq!(env.refs(), refs0);
}
