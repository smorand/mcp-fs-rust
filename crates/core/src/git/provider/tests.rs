//! US-024 acceptance tests: resolution, the injectable seam, and the transport
//! safety rules. Every test runs offline: the "provider" is an axum router on
//! an ephemeral loopback port.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use serde_json::json;

use super::*;
use crate::config::GitConfig;
use crate::errors::code;

// ── host map fixture ────────────────────────────────────────────────────────

const HOSTS: &[(&str, &str)] = &[
    ("github.com", "github"),
    ("github.ibm.com", "github"),
    ("gitlab.com", "gitlab"),
    ("gitlab.example.test", "gitlab"),
    ("git.acme.internal", "generic"),
    ("public.example.org", "anonymous"),
];

/// The published `git.hosts` map is a process wide static, so every test that
/// reads it serializes through the same lock the other suites use.
fn with_hosts<T>(f: impl FnOnce() -> T) -> T {
    let _guard = crate::git::remote::tests::lock_for_test();
    let mut cfg = GitConfig::default();
    cfg.hosts.0 = HOSTS.iter().map(|(h, p)| ((*h).to_string(), (*p).to_string())).collect();
    crate::git::remote::validate_hosts(&cfg).expect("a valid host map");
    f()
}

// ── fake provider server ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    path: String,
    authorization: Option<String>,
    host: String,
}

#[derive(Clone)]
struct Canned {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

impl Canned {
    fn json(status: u16, body: serde_json::Value) -> Self {
        Self { status, location: None, body: body.to_string().into_bytes() }
    }
    fn redirect(status: u16, location: impl Into<String>) -> Self {
        Self { status, location: Some(location.into()), body: Vec::new() }
    }
    fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self { status, location: None, body }
    }
}

#[derive(Clone, Default)]
struct FakeProvider {
    calls: Arc<Mutex<Vec<Recorded>>>,
    routes: Arc<Mutex<HashMap<String, Vec<Canned>>>>,
}

impl FakeProvider {
    fn route(&self, path: &str, responses: Vec<Canned>) {
        self.routes.lock().unwrap().insert(path.to_string(), responses);
    }
    fn calls(&self) -> Vec<Recorded> {
        self.calls.lock().unwrap().clone()
    }
}

async fn handle(State(state): State<FakeProvider>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_string();
    state.calls.lock().unwrap().push(Recorded {
        method: req.method().to_string(),
        path: path.clone(),
        authorization: req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        host: req.headers().get("host").and_then(|v| v.to_str().ok()).unwrap_or("").to_string(),
    });
    let canned = {
        let mut routes = state.routes.lock().unwrap();
        match routes.get_mut(&path) {
            Some(queue) if queue.len() > 1 => queue.remove(0),
            Some(queue) if !queue.is_empty() => queue[0].clone(),
            _ => Canned::json(404, json!({"message": "no route"})),
        }
    };
    let mut builder = Response::builder().status(StatusCode::from_u16(canned.status).unwrap());
    if let Some(loc) = canned.location {
        builder = builder.header("location", loc);
    }
    builder.body(Body::from(canned.body)).unwrap()
}

/// Spawn the fake on an ephemeral port. Returns its base URL and the recorder.
async fn spawn_provider() -> (String, FakeProvider) {
    let state = FakeProvider::default();
    let app = axum::Router::new().fallback(axum::routing::any(handle)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), state)
}

fn target_at(base: &str, host: &str, provider: PrProvider) -> ProviderTarget {
    ProviderTarget {
        host: host.to_string(),
        provider,
        base_url: base.to_string(),
        owner: "acme".into(),
        repo: "api".into(),
    }
}

const GH_TOKEN: &str = "ghp_TESTTOKEN_github_001";
const GL_TOKEN: &str = "glpat_TESTTOKEN_gl_003";

fn client() -> HttpProviderClient {
    HttpProviderClient::new(30).unwrap()
}

// ── resolution: FR-NEW-300/301/302 ──────────────────────────────────────────

/// E2E-NEW-706: a host absent from `git.hosts` is refused before any call.
#[test]
fn e2e_new_706_an_undeclared_host_is_rejected() {
    with_hosts(|| {
        let err = resolve_target("https://git.sourcehut.test/acme/api.git", None)
            .expect_err("an undeclared host has no provider");
        assert!(
            err.message.contains("host 'git.sourcehut.test' is not declared in git.hosts"),
            "unexpected message: {}",
            err.message
        );
    });
}

/// E2E-NEW-708: the scp shorthand is refused, naming the url and `scp`.
#[test]
fn e2e_new_708_the_scp_shorthand_remote_is_rejected() {
    with_hosts(|| {
        let err = resolve_target("git@github.com:acme/api.git", None)
            .expect_err("the scp shorthand is not a supported remote url");
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(
            err.message.contains("remote url 'git@github.com:acme/api.git'"),
            "{}",
            err.message
        );
        assert!(err.message.contains("scp"), "{}", err.message);
    });
}

/// E2E-NEW-923: `generic` and `anonymous` are refused with `ERR_NOT_SUPPORTED`,
/// and the refusal happens inside resolution, which reads no token at all: the
/// provider check therefore precedes both the token lookup and the scope check.
#[test]
fn e2e_new_923_an_unsupported_provider_is_rejected_before_any_token_is_read() {
    with_hosts(|| {
        for (url, host) in [
            ("https://git.acme.internal/acme/api.git", "git.acme.internal"),
            ("https://public.example.org/acme/api.git", "public.example.org"),
        ] {
            let err = resolve_target(url, None).expect_err("provider has no pull request API");
            assert_eq!(err.code, code::NOT_SUPPORTED, "{}", err.message);
            assert_ne!(err.code, code::FORBIDDEN);
            assert_ne!(err.code, code::UNAUTHENTICATED);
            for needle in [host, "github", "gitlab"] {
                assert!(err.message.contains(needle), "{needle} missing from {}", err.message);
            }
        }
    });
}

/// E2E-NEW-924: a parseable URL carrying no repository slug is refused.
#[test]
fn e2e_new_924_a_remote_with_no_project_path_is_rejected() {
    with_hosts(|| {
        for url in ["https://github.com/", "https://github.com/acme"] {
            let err = resolve_target(url, None).expect_err("no repository in the path");
            assert_eq!(err.code, code::INVALID_ARGUMENT);
            assert!(err.message.contains(url), "{}", err.message);
            assert!(err.message.contains("repository"), "{}", err.message);
            assert!(!err.message.contains("ghp_"), "{}", err.message);
        }
    });
}

/// FR-NEW-300: github.com, GitHub Enterprise Server and self hosted GitLab each
/// resolve their API base with no new configuration.
#[test]
fn fr_new_300_api_base_resolves_from_the_host_class() {
    with_hosts(|| {
        let gh = resolve_target("https://github.com/acme/api.git", None).unwrap();
        assert_eq!(gh.base_url, "https://api.github.com");
        assert_eq!(gh.provider, PrProvider::Github);
        assert_eq!(gh.project_path(), "acme/api");

        let ghe = resolve_target("https://github.ibm.com/acme/api.git", None).unwrap();
        assert_eq!(ghe.base_url, "https://github.ibm.com/api/v3");

        let gl = resolve_target("https://gitlab.com/acme/api.git", None).unwrap();
        assert_eq!(gl.base_url, "https://gitlab.com/api/v4");
        assert_eq!(gl.provider, PrProvider::Gitlab);

        // The stored instance_url wins for a self hosted GitLab whose API base
        // is not the git origin.
        let self_hosted = resolve_target(
            "https://gitlab.example.test/acme/group/api.git",
            Some("https://gitlab.example.test/gl/"),
        )
        .unwrap();
        assert_eq!(self_hosted.base_url, "https://gitlab.example.test/gl/api/v4");
        assert_eq!(self_hosted.project_path(), "acme/group/api");
    });
}

// ── the seam: FR-NEW-315 ────────────────────────────────────────────────────

/// FR-NEW-315: a test replaces the transport with a fake returning canned JSON,
/// and the fake sees the request the production code would have sent.
#[tokio::test]
async fn fr_new_315_the_provider_client_is_injectable() {
    struct Fake {
        seen: Mutex<Vec<ProviderRequest>>,
    }
    #[async_trait]
    impl ProviderClient for Fake {
        async fn send(&self, req: &ProviderRequest, _c: &Credential) -> Result<ProviderResponse> {
            self.seen.lock().unwrap().push(req.clone());
            Ok(ProviderResponse::complete(200, json!({"number": 42}).to_string().into_bytes()))
        }
    }
    let fake: Arc<dyn ProviderClient> = Arc::new(Fake { seen: Mutex::new(Vec::new()) });
    let target = target_at("https://api.github.com", "github.com", PrProvider::Github);
    let req = ProviderRequest::new(&target, Method::Get, "/repos/acme/api/pulls/42");
    let resp = fake.send(&req, &Credential::new(PrProvider::Github, GH_TOKEN)).await.unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json().unwrap()["number"], 42);
}

// ── transport safety: FR-NEW-314/316 ────────────────────────────────────────

/// E2E-NEW-792: GitHub gets `token <t>`, GitLab gets `Bearer <t>`, and the
/// header is present on every request issued, sub-calls included.
#[tokio::test]
async fn e2e_new_792_the_authorization_scheme_is_per_provider() {
    let (base, fake) = spawn_provider().await;
    fake.route("/repos/acme/api/pulls/42", vec![Canned::json(200, json!({"number": 42}))]);
    fake.route("/repos/acme/api/pulls/42/reviews", vec![Canned::json(200, json!([]))]);
    fake.route("/api/v4/projects/acme%2Fapi/merge_requests/42", vec![Canned::json(200, json!({}))]);

    let c = client();
    let gh = target_at(&base, "github.com", PrProvider::Github);
    let gh_cred = Credential::new(PrProvider::Github, GH_TOKEN);
    for path in ["/repos/acme/api/pulls/42", "/repos/acme/api/pulls/42/reviews"] {
        let r = c.send(&ProviderRequest::new(&gh, Method::Get, path), &gh_cred).await.unwrap();
        assert_eq!(r.status, 200);
    }

    let gl = target_at(&format!("{base}/api/v4"), "gitlab.com", PrProvider::Gitlab);
    let gl_cred = Credential::new(PrProvider::Gitlab, GL_TOKEN);
    let r = c
        .send(
            &ProviderRequest::new(&gl, Method::Get, "/projects/acme%2Fapi/merge_requests/42"),
            &gl_cred,
        )
        .await
        .unwrap();
    assert_eq!(r.status, 200);

    let calls = fake.calls();
    assert_eq!(calls.len(), 3);
    for call in &calls[..2] {
        assert_eq!(call.method, "GET");
        assert_eq!(call.authorization.as_deref(), Some(&*format!("token {GH_TOKEN}")));
    }
    assert_eq!(calls[2].authorization.as_deref(), Some(&*format!("Bearer {GL_TOKEN}")));
}

/// E2E-NEW-791: two people hold a token on the SAME host; neither request ever
/// carries the other person's value. The credential is resolved through the
/// US-023 gate, the only entry point for a token plus its scope check.
#[tokio::test]
async fn e2e_new_791_two_people_on_one_host_never_cross_credentials() {
    use crate::git::oauth::scopes::PrAccess;
    let (base, fake) = spawn_provider().await;
    fake.route("/repos/acme/api/pulls", vec![Canned::json(200, json!([]))]);

    let store = crate::git::OAuthTokenStore::new();
    for (person, token) in [("dev@test.com", GH_TOKEN), ("alice@test.com", "ghp_ALICE_777")] {
        store
            .store_token(person, "github.com", "github", token, vec!["repo".into()], None, None)
            .await
            .unwrap();
    }

    let c = client();
    let target = target_at(&base, "github.com", PrProvider::Github);
    for person in ["dev@test.com", "alice@test.com"] {
        let token = store.require_pr_credential(person, "github.com", PrAccess::Read).unwrap();
        let cred = Credential::new(PrProvider::Github, token);
        c.send(&ProviderRequest::new(&target, Method::Get, "/repos/acme/api/pulls"), &cred)
            .await
            .unwrap();
    }

    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].authorization.as_deref(), Some(&*format!("token {GH_TOKEN}")));
    assert_eq!(calls[1].authorization.as_deref(), Some("token ghp_ALICE_777"));
    assert!(calls[0].authorization.as_deref() != Some("token ghp_ALICE_777"));
    assert!(calls[1].authorization.as_deref() != Some(&*format!("token {GH_TOKEN}")));
}

/// E2E-NEW-786: a 401 whose body deliberately echoes the credential comes back
/// scrubbed, so nothing downstream can put it in a response or a message.
#[tokio::test]
async fn e2e_new_786_a_provider_body_echoing_the_credential_is_scrubbed() {
    let (base, _fake) = spawn_provider().await;
    let c = client();

    for (provider, token, body) in [
        (PrProvider::Github, GH_TOKEN, json!({"message": format!("Bad credentials: {GH_TOKEN}")})),
        (
            PrProvider::Gitlab,
            GL_TOKEN,
            json!({"message": format!("401 Unauthorized for {GL_TOKEN}")}),
        ),
    ] {
        let (base2, fake2) = spawn_provider().await;
        let _ = &base;
        fake2.route("/unauthorized", vec![Canned::json(401, body)]);
        let target = target_at(&base2, "github.com", provider);
        let resp = c
            .send(
                &ProviderRequest::new(&target, Method::Get, "/unauthorized"),
                &Credential::new(provider, token),
            )
            .await
            .unwrap();
        assert_eq!(resp.status, 401);
        let text = resp.text();
        assert!(!text.contains(token), "token leaked in body: {text}");
        assert!(!text.contains("ghp_"), "{text}");
        assert!(!text.contains("glpat_"), "{text}");
    }
}

/// E2E-NEW-787: with a subscriber capturing every span and field, no captured
/// text carries a token prefix, on both the success and the failure path.
#[test]
fn e2e_new_787_no_span_or_field_ever_carries_a_token() {
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Captured;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut observed = Vec::new();
    rt.block_on(async {
        let (base, fake) = spawn_provider().await;
        fake.route("/ok", vec![Canned::json(200, json!({"number": 42}))]);
        fake.route(
            "/bad",
            vec![Canned::json(401, json!({"message": format!("Bad credentials: {GH_TOKEN}")}))],
        );
        let c = client();
        let target = target_at(&base, "github.com", PrProvider::Github);
        let cred = Credential::new(PrProvider::Github, GH_TOKEN);
        for path in ["/ok", "/bad", "/missing"] {
            match c.send(&ProviderRequest::new(&target, Method::Get, path), &cred).await {
                Ok(r) => observed.push(r.text()),
                Err(e) => observed.push(e.message),
            }
        }
        // A credential must also be safe to put in any diagnostic verbatim.
        observed.push(format!("{cred:?}"));
    });
    drop(guard);

    observed.push(String::from_utf8_lossy(&captured.0.lock().unwrap()).into_owned());
    for text in observed {
        assert!(!text.contains("ghp_"), "token prefix leaked: {text}");
        assert!(!text.contains(GH_TOKEN), "token leaked: {text}");
    }
}

/// E2E-NEW-794: a cross host redirect is refused, naming both hosts, and the
/// credential is never re-sent: exactly one request leaves this process.
#[tokio::test]
async fn e2e_new_794_a_cross_host_redirect_is_refused() {
    let (base, fake) = spawn_provider().await;
    fake.route(
        "/repos/acme/api/pulls/42",
        vec![Canned::redirect(302, "https://evil.test/repos/acme/api/pulls/42")],
    );
    let target = target_at(&base, "github.com", PrProvider::Github);
    let err = client()
        .send(
            &ProviderRequest::new(&target, Method::Get, "/repos/acme/api/pulls/42"),
            &Credential::new(PrProvider::Github, GH_TOKEN),
        )
        .await
        .expect_err("a cross host redirect must fail the call");
    assert_eq!(err.code, code::INTERNAL_ERROR);
    assert!(
        err.message.contains("refused redirect from host '127.0.0.1' to 'evil.test'"),
        "{}",
        err.message
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "the credential must not be replayed: {calls:?}");
    assert!(!calls.iter().any(|c| c.host.contains("evil.test")));
}

/// E2E-NEW-927: the rule is "cross host", not "no redirects": a same host
/// redirect is followed, with the credential, to the new path.
#[tokio::test]
async fn e2e_new_927_a_same_host_redirect_is_followed() {
    let (base, fake) = spawn_provider().await;
    fake.route(
        "/repos/acme/api/pulls/42",
        vec![Canned::redirect(301, format!("{base}/repositories/99/pulls/42"))],
    );
    fake.route("/repositories/99/pulls/42", vec![Canned::json(200, json!({"number": 42}))]);

    let target = target_at(&base, "github.com", PrProvider::Github);
    let resp = client()
        .send(
            &ProviderRequest::new(&target, Method::Get, "/repos/acme/api/pulls/42"),
            &Credential::new(PrProvider::Github, GH_TOKEN),
        )
        .await
        .unwrap();
    assert_eq!(resp.json().unwrap()["number"], 42);

    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].path, "/repositories/99/pulls/42");
    assert_eq!(calls[1].authorization.as_deref(), Some(&*format!("token {GH_TOKEN}")));
}

/// E2E-NEW-795: an oversized JSON body is refused rather than buffered, and the
/// refusal comes back fast.
#[tokio::test]
async fn e2e_new_795_an_oversized_response_is_refused() {
    let (base, fake) = spawn_provider().await;
    // 20 MiB, comfortably past the 16 MiB ceiling without making the suite slow.
    let huge = vec![b'x'; 20 * 1024 * 1024];
    fake.route("/repos/acme/api/pulls", vec![Canned::bytes(200, huge)]);

    let target = target_at(&base, "github.com", PrProvider::Github);
    let c = client();
    let req = ProviderRequest::new(&target, Method::Get, "/repos/acme/api/pulls");
    let cred = Credential::new(PrProvider::Github, GH_TOKEN);
    let call = c.send(&req, &cred);
    let err = tokio::time::timeout(std::time::Duration::from_secs(10), call)
        .await
        .expect("the body must be capped, not fully buffered")
        .expect_err("an oversized body must be refused");
    assert_eq!(err.code, code::INVALID_ARGUMENT);
    assert!(
        err.message.contains("response from host 'github.com' exceeds the 16 MiB limit"),
        "{}",
        err.message
    );
}

/// FR-NEW-316 / NFR 7.1: the timeout comes from config, and the shared client
/// is built exactly once so its connection pool is reused.
#[test]
fn provider_api_timeout_is_configured_and_the_client_is_shared() {
    let cfg = GitConfig::default();
    assert_eq!(cfg.provider_api_timeout_secs, 30);
    let a = shared_client(&cfg).unwrap();
    let b = shared_client(&cfg).unwrap();
    assert!(Arc::ptr_eq(&a, &b), "the provider client must be built once and shared");
}
