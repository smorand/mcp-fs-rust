//! `git.auth*` tools: OAuth device flow for GitHub and GitLab.
//!
//! Port of the C# `Tools/GitAuthTools.cs`. Registered only when `git.enabled`.
//!
//! No project is involved, so the only gate is an authenticated caller: a token
//! belongs to the person, not to a mount. `git.auth` returns as soon as the
//! provider hands out a user code, then a background task polls the token
//! endpoint and stores the result in the [`OAuthTokenStore`], which is the same
//! store `git.remote_clone` reads from (hence the process singleton below).

use crate::config::ServerConfig;
use crate::errors::{Result, ToolError};
use crate::git::oauth::device_flow::{DeviceCode, DeviceFlowClient, HttpDeviceFlowClient};
use crate::git::oauth::store::OAuthTokenStore;
use crate::mcp::registry::{ToolCtx, handler};
use crate::mcp::{ToolRegistry, ToolSchema};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// The two providers the device flow supports, in the order `git.auth_status`
/// reports them when no provider is given.
pub const PROVIDERS: [&str; 2] = ["github", "gitlab"];

/// The bound `git.token_set` enforces on a seeded token (DEC-033): a
/// denial-of-service bound, matching `TextKey`'s length ceiling on the
/// persisted column (`crates/mcp-fs/src/git/oauth/persistence.rs:33-34`).
const MAX_TOKEN_LEN: usize = 8192;

/// A provider that returns `interval: 0` must not turn the poll loop into a spin.
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Process wide token store, built once from the config. `git.auth` writes it and
/// `git.remote_clone` reads it, so they must be the same instance; the error is
/// cached too, because a malformed `MCPFS_TOKEN_KEY` must fail loudly every time
/// instead of silently downgrading to memory only storage.
/// A `tokio::sync::OnceCell` rather than a `OnceLock`: building the store now
/// loads the encrypted rows through an async driver, so initialisation awaits.
static TOKENS: tokio::sync::OnceCell<Result<Arc<OAuthTokenStore>>> =
    tokio::sync::OnceCell::const_new();

/// Process wide device flow client. Reuses one `reqwest` connection pool.
static FLOW: OnceLock<Result<Arc<dyn DeviceFlowClient>>> = OnceLock::new();

pub async fn token_store(
    config: &ServerConfig,
    registry: &crate::storage::RelationalRegistry,
) -> Result<Arc<OAuthTokenStore>> {
    TOKENS
        .get_or_init(|| async { OAuthTokenStore::from_env(config, registry).await.map(Arc::new) })
        .await
        .clone()
}

fn device_flow(config: &ServerConfig) -> Result<Arc<dyn DeviceFlowClient>> {
    FLOW.get_or_init(|| {
        HttpDeviceFlowClient::new(config.git.clone())
            .map(|c| Arc::new(c) as Arc<dyn DeviceFlowClient>)
    })
    .clone()
}

/// Register the three `git.auth*` tools.
pub fn register(reg: &mut ToolRegistry) {
    register_with(reg, None, None);
}

/// Registration with injected dependencies, for tests: a token store that is not
/// the process singleton and a fake device flow that never reaches the network.
pub fn register_with(
    reg: &mut ToolRegistry,
    tokens: Option<Arc<OAuthTokenStore>>,
    flow: Option<Arc<dyn DeviceFlowClient>>,
) {
    let (t, f) = (tokens.clone(), flow);
    reg.add(
        ToolSchema::new(
            "git.auth",
            "Start OAuth device flow for GitHub or GitLab. Returns user_code and verification_uri.",
        )
        .req_str("provider", "OAuth provider: github or gitlab.")
        .opt_str_null(
            "instance_url",
            "Optional self-hosted instance URL (e.g. GitLab Enterprise).",
        ),
        handler(move |ctx: ToolCtx, a| {
            let (t, f) = (t.clone(), f.clone());
            async move {
                let provider = a.str("provider")?;
                let instance_url = a.opt_str("instance_url");
                let tokens = resolve_tokens(&ctx, t).await?;
                let flow = match f {
                    Some(f) => f,
                    None => device_flow(&ctx.state.config)?,
                };
                auth(&ctx, &provider, instance_url, tokens, flow).await
            }
        }),
    );

    let t = tokens.clone();
    reg.add(
        ToolSchema::new(
            "git.auth_status",
            "Check authentication status for a provider (or all providers).",
        )
        .opt_str_null(
            "provider",
            "Provider to check: github or gitlab; omit to report all providers.",
        ),
        handler(move |ctx: ToolCtx, a| {
            let t = t.clone();
            async move {
                let tokens = resolve_tokens(&ctx, t).await?;
                auth_status(&ctx, a.opt_str("provider").as_deref(), &tokens)
            }
        }),
    );

    let t = tokens.clone();
    reg.add(
        ToolSchema::new("git.auth_revoke", "Revoke the stored token for a provider.")
            .req_str("provider", "Provider whose stored token is revoked: github or gitlab."),
        handler(move |ctx: ToolCtx, a| {
            let t = t.clone();
            async move {
                let provider = a.str("provider")?;
                let tokens = resolve_tokens(&ctx, t).await?;
                auth_revoke(&ctx, &provider, &tokens).await
            }
        }),
    );

    let t = tokens;
    reg.add(
        ToolSchema::new(
            "git.token_set",
            "Seed a personal access token you already hold for a host declared in git.hosts, \
             without the interactive device flow. The token is never echoed back.",
        )
        .req_str("host", "Hostname declared in git.hosts to store the token for.")
        .req_str(
            "token",
            "The personal access token value. Never echoed back; 1 to 8192 characters.",
        )
        .opt_str_null(
            "expires_at",
            "RFC 3339 timestamp the token expires at; omit or null for a token that never \
             expires.",
        ),
        handler(move |ctx: ToolCtx, a| {
            let t = t.clone();
            async move {
                let host = a.str("host")?;
                let token = a.str("token")?;
                let expires_at = match a.raw("expires_at") {
                    None => None,
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(_) => {
                        return Err(ToolError::invalid_argument(
                            "argument 'expires_at' must be an RFC 3339 timestamp string",
                        ));
                    }
                };
                let tokens = resolve_tokens(&ctx, t).await?;
                token_set(&ctx, &host, &token, expires_at, &tokens).await
            }
        }),
    );
}

async fn resolve_tokens(
    ctx: &ToolCtx,
    injected: Option<Arc<OAuthTokenStore>>,
) -> Result<Arc<OAuthTokenStore>> {
    match injected {
        Some(t) => Ok(t),
        None => token_store(&ctx.state.config, ctx.state.stores.relational()).await,
    }
}

/// The caller identity. Empty means the request reached a tool without passing
/// identity verification, which is a bug, not a permission problem.
fn require_identity(ctx: &ToolCtx) -> Result<String> {
    if ctx.person.trim().is_empty() {
        return Err(ToolError::unauthenticated("no authenticated identity in context"));
    }
    Ok(ctx.person.clone())
}

// ── implementations ─────────────────────────────────────────────────────────

async fn auth(
    ctx: &ToolCtx,
    provider: &str,
    instance_url: Option<String>,
    tokens: Arc<OAuthTokenStore>,
    flow: Arc<dyn DeviceFlowClient>,
) -> Result<Value> {
    // Exact match, like the C#: "GitHub" is a client bug worth reporting.
    if provider != "github" && provider != "gitlab" {
        return Err(ToolError::invalid_argument("provider must be 'github' or 'gitlab'"));
    }
    let person = require_identity(ctx)?;
    let code = flow.request_device_code(provider, instance_url.as_deref()).await?;

    let message = format!("Open {} and enter code {}", code.verification_uri, code.user_code);
    let result = json!({
        "status": "pending",
        "provider": provider,
        "user_code": code.user_code,
        "verification_uri": code.verification_uri,
        "expires_in": code.expires_in,
        "message": message,
    });

    spawn_poller(flow, tokens, person, provider.to_string(), instance_url, code);
    Ok(result)
}

/// Poll the token endpoint until the user authorizes, refuses, or the code dies.
/// Detached on purpose: `git.auth` must answer immediately so the caller can show
/// the code, and the client then polls `git.auth_status`.
fn spawn_poller(
    flow: Arc<dyn DeviceFlowClient>,
    tokens: Arc<OAuthTokenStore>,
    person: String,
    provider: String,
    instance_url: Option<String>,
    code: DeviceCode,
) {
    let interval = Duration::from_secs(code.interval.max(0) as u64).max(MIN_POLL_INTERVAL);
    let lifetime = Duration::from_secs(code.expires_in.max(0) as u64);
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + lifetime;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(interval).await;
            match flow.poll_for_token(&code).await {
                Ok(poll) if poll.success => {
                    let Some(token) = poll.access_token else { return };
                    // `git.auth` only ever hands this poller a provider name, not a
                    // hostname (its tool schema is unchanged by this story: US-006
                    // and US-007 carry the real per-host wiring). Using the provider
                    // as the host here is a like-for-like continuation of what this
                    // call site already did before the store's key became
                    // `(person, host)`, not a new design decision.
                    if let Err(e) = tokens
                        .store_token(
                            &person,
                            &provider,
                            &provider,
                            &token,
                            poll.scopes,
                            Some(poll.expires_at),
                            instance_url,
                        )
                        .await
                    {
                        // The message never contains the token itself.
                        tracing::warn!("git.auth: cannot store the {provider} token: {e}");
                    }
                    return;
                }
                Ok(poll) if poll.is_terminal_error() => return,
                // authorization_pending, slow_down, or a transient transport
                // error: the user may still be typing the code.
                _ => {}
            }
        }
    });
}

fn auth_status(ctx: &ToolCtx, provider: Option<&str>, tokens: &OAuthTokenStore) -> Result<Value> {
    let person = require_identity(ctx)?;
    if let Some(p) = provider {
        return Ok(status_for(&person, p, tokens, true));
    }
    let statuses: Vec<Value> =
        PROVIDERS.iter().map(|p| status_for(&person, p, tokens, false)).collect();
    Ok(json!({"statuses": statuses}))
}

/// One provider's status. `single` reproduces the C# key order difference between
/// the single provider answer (`authenticated` first) and the list entries
/// (`provider` first).
fn status_for(person: &str, provider: &str, tokens: &OAuthTokenStore, single: bool) -> Value {
    let authenticated = tokens.has_valid_token(person, provider);
    let mut out = serde_json::Map::new();
    if single {
        out.insert("authenticated".into(), json!(authenticated));
        out.insert("provider".into(), json!(provider));
    } else {
        out.insert("provider".into(), json!(provider));
        out.insert("authenticated".into(), json!(authenticated));
    }
    if authenticated && let Some(s) = tokens.get_token(person, provider) {
        out.insert("scopes".into(), json!(s.scopes));
        // `None` (a non-expiring token, the Implementer Decision in US-003) has
        // no caller yet: every store_token call site in this file still passes
        // `Some`. Handled here only because the type is now `Option`.
        out.insert("expires_at".into(), json!(s.expires_at.map(round_trip_iso)));
    }
    Value::Object(out)
}

async fn auth_revoke(ctx: &ToolCtx, provider: &str, tokens: &OAuthTokenStore) -> Result<Value> {
    let person = require_identity(ctx)?;
    tokens.revoke_token(&person, provider).await?;
    Ok(json!({"provider": provider, "revoked": true}))
}

/// `git.token_set` (FR-NEW-013): store a token the caller already holds,
/// instead of forcing the interactive device flow for a credential the caller
/// already possesses. The provider is resolved from `git.hosts`
/// (`crate::git::remote::resolve_host`), never supplied by the caller.
async fn token_set(
    ctx: &ToolCtx,
    host: &str,
    token: &str,
    expires_at: Option<String>,
    tokens: &OAuthTokenStore,
) -> Result<Value> {
    let person = require_identity(ctx)?;

    use crate::git::remote::Provider;
    let provider = crate::git::remote::resolve_host(host).map_err(|_| {
        ToolError::invalid_argument(format!(
            "host '{host}' is not declared in git.hosts; declare it under git.hosts before \
             seeding a token for it"
        ))
    })?;
    if provider == Provider::Anonymous {
        return Err(ToolError::invalid_argument(format!(
            "host '{host}' is declared anonymous: an anonymous host holds no credential"
        )));
    }
    let provider_name = match provider {
        Provider::Github => "github",
        Provider::Gitlab => "gitlab",
        Provider::Generic => "generic",
        Provider::Anonymous => unreachable!("handled above"),
    };

    if token.trim().is_empty() {
        return Err(ToolError::invalid_argument("token must not be empty or whitespace-only"));
    }
    if token.chars().count() > MAX_TOKEN_LEN {
        return Err(ToolError::invalid_argument(format!(
            "token must be at most {MAX_TOKEN_LEN} characters"
        )));
    }

    // A past expiry is accepted (DEC-017, FR-NEW-052): the token is simply
    // already expired at seed time, reported so by `git.auth_status` rather
    // than rejected here.
    let expires_at = match expires_at {
        Some(s) => Some(
            DateTime::parse_from_rfc3339(&s)
                .map_err(|_| {
                    ToolError::invalid_argument(format!(
                        "argument 'expires_at' must be an RFC 3339 timestamp, got '{s}'"
                    ))
                })?
                .with_timezone(&Utc),
        ),
        None => None,
    };

    tokens.store_token(&person, host, provider_name, token, Vec::new(), expires_at, None).await?;

    let persistent = tokens.is_persistent();
    let mut out = json!({
        "host": host,
        "provider": provider_name,
        "stored": true,
        "persistent": persistent,
    });
    if !persistent {
        out["message"] = json!(
            "Stored in memory only: this token lives only for the current process and will \
             not survive a restart because MCPFS_TOKEN_KEY is not configured."
        );
    }
    Ok(out)
}

/// The C# `DateTimeOffset.ToString("O")`: seven fractional digits (100ns ticks)
/// plus an explicit offset, which is always UTC here.
fn round_trip_iso(dt: DateTime<Utc>) -> String {
    format!("{}{:07}+00:00", dt.format("%Y-%m-%dT%H:%M:%S."), dt.timestamp_subsec_nanos() / 100)
}

#[cfg(test)]
mod tests {
    use super::super::admin::test_support::Fixture;
    use super::*;
    use crate::errors::code;
    use crate::git::oauth::device_flow::TokenPoll;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PERSON: &str = "dev@test.com";

    /// The C# `IDeviceFlowClient` test double: a canned device code plus a script
    /// of poll answers, the last one repeating.
    struct FakeFlow {
        code: DeviceCode,
        script: Mutex<Vec<TokenPoll>>,
        polls: AtomicUsize,
        requests: Mutex<Vec<(String, Option<String>)>>,
    }

    impl FakeFlow {
        fn new(script: Vec<TokenPoll>) -> Arc<Self> {
            Arc::new(Self {
                code: DeviceCode {
                    provider: "github".into(),
                    device_code: "device-secret".into(),
                    user_code: "WXYZ-9876".into(),
                    verification_uri: "https://github.com/login/device".into(),
                    // 5 seconds is plenty: the interval floor is 10 ms in tests.
                    expires_in: 5,
                    interval: 0,
                    instance_url: None,
                },
                script: Mutex::new(script),
                polls: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
            })
        }

        fn polls(&self) -> usize {
            self.polls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DeviceFlowClient for FakeFlow {
        async fn request_device_code(
            &self,
            provider: &str,
            instance_url: Option<&str>,
        ) -> Result<DeviceCode> {
            self.requests
                .lock()
                .unwrap()
                .push((provider.to_string(), instance_url.map(str::to_string)));
            Ok(DeviceCode { provider: provider.to_string(), ..self.code.clone() })
        }

        async fn poll_for_token(&self, _code: &DeviceCode) -> Result<TokenPoll> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            let mut q = self.script.lock().unwrap();
            Ok(if q.len() > 1 { q.remove(0) } else { q[0].clone() })
        }
    }

    fn future() -> DateTime<Utc> {
        Utc::now() + chrono::Duration::hours(1)
    }

    /// Fixture plus an isolated token store and a fake flow.
    async fn setup(
        script: Vec<TokenPoll>,
    ) -> (Fixture, ToolRegistry, Arc<OAuthTokenStore>, Arc<FakeFlow>) {
        let f = Fixture::with_config(|c| c.git.enabled = true).await;
        let tokens = Arc::new(OAuthTokenStore::new());
        let flow = FakeFlow::new(script);
        let mut r = ToolRegistry::new();
        register_with(&mut r, Some(tokens.clone()), Some(flow.clone()));
        (f, r, tokens, flow)
    }

    /// Wait (bounded) for a condition the background poller drives.
    async fn eventually(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn every_git_auth_tool_is_registered() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        assert_eq!(r.len(), 4);
        for name in ["git.auth", "git.auth_status", "git.auth_revoke", "git.token_set"] {
            assert!(r.resolve(name).is_some(), "{name} is missing");
        }
    }

    #[tokio::test]
    async fn git_auth_schema_matches_the_contract() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.auth").unwrap().schema;
        assert_eq!(
            s.description,
            "Start OAuth device flow for GitHub or GitLab. Returns user_code and verification_uri."
        );
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "provider":{"description":"OAuth provider: github or gitlab.","type":"string"},
                 "instance_url":{"description":"Optional self-hosted instance URL (e.g. GitLab Enterprise).","type":"string","default":null}},
               "required":["provider"]}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);
    }

    #[tokio::test]
    async fn git_auth_status_schema_has_no_required_parameter() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.auth_status").unwrap().schema;
        assert_eq!(s.description, "Check authentication status for a provider (or all providers).");
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "provider":{"description":"Provider to check: github or gitlab; omit to report all providers.","type":"string","default":null}}}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);

        let rev = &r.resolve("git.auth_revoke").unwrap().schema;
        assert_eq!(rev.description, "Revoke the stored token for a provider.");
        assert_eq!(rev.input_schema()["required"], json!(["provider"]));
    }

    #[tokio::test]
    async fn git_auth_returns_pending_immediately() {
        let (f, r, _tokens, flow) = setup(vec![TokenPoll::pending("authorization_pending")]).await;
        let out = f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();
        assert_eq!(out["status"], "pending");
        assert_eq!(out["provider"], "github");
        assert_eq!(out["user_code"], "WXYZ-9876");
        assert_eq!(out["verification_uri"], "https://github.com/login/device");
        assert_eq!(out["expires_in"], 5);
        assert_eq!(out["message"], "Open https://github.com/login/device and enter code WXYZ-9876");
        assert_eq!(flow.requests.lock().unwrap()[0], ("github".to_string(), None));
    }

    #[tokio::test]
    async fn pending_then_success_stores_the_token() {
        let (f, r, tokens, flow) = setup(vec![
            TokenPoll::pending("authorization_pending"),
            TokenPoll::granted("gho_stored", vec!["repo".into()], future()),
        ])
        .await;
        f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();

        assert!(
            eventually(|| tokens.has_valid_token(PERSON, "github")).await,
            "the poller must store the token once the user authorizes"
        );
        assert!(flow.polls() >= 2, "the pending answer must not end the loop");
        let s = tokens.get_token(PERSON, "github").unwrap();
        assert_eq!(s.access_token, "gho_stored");
        assert_eq!(s.scopes, vec!["repo"]);
        assert!(s.instance_url.is_none());
    }

    #[tokio::test]
    async fn a_refused_authorization_stores_nothing_and_stops_polling() {
        let (f, r, tokens, flow) = setup(vec![TokenPoll::pending("access_denied")]).await;
        f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();

        assert!(eventually(|| flow.polls() >= 1).await, "the poller must run at least once");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(tokens.get_token(PERSON, "github").is_none(), "no token on refusal");
        let after = flow.polls();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(flow.polls(), after, "access_denied is terminal");
    }

    #[tokio::test]
    async fn gitlab_keeps_the_instance_url_on_the_stored_session() {
        let (f, r, tokens, flow) =
            setup(vec![TokenPoll::granted("glpat", vec!["api".into()], future())]).await;
        f.call(
            &r,
            PERSON,
            "git.auth",
            json!({"provider":"gitlab","instance_url":"https://gitlab.example.test"}),
        )
        .await
        .unwrap();

        assert!(eventually(|| tokens.has_valid_token(PERSON, "gitlab")).await);
        let s = tokens.get_token(PERSON, "gitlab").unwrap();
        assert_eq!(s.instance_url.as_deref(), Some("https://gitlab.example.test"));
        assert_eq!(
            flow.requests.lock().unwrap()[0],
            ("gitlab".to_string(), Some("https://gitlab.example.test".to_string()))
        );
    }

    #[tokio::test]
    async fn an_unknown_provider_is_rejected_before_any_http_call() {
        let (f, r, _tokens, flow) = setup(vec![TokenPoll::pending("x")]).await;
        for bad in ["bitbucket", "GitHub", ""] {
            let e = f.call(&r, PERSON, "git.auth", json!({"provider":bad})).await.unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT, "'{bad}' must be rejected");
            assert_eq!(e.message, "provider must be 'github' or 'gitlab'");
        }
        assert!(flow.requests.lock().unwrap().is_empty(), "no device code was requested");
    }

    #[tokio::test]
    async fn auth_status_reports_one_provider() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        let out =
            f.call(&r, PERSON, "git.auth_status", json!({"provider":"github"})).await.unwrap();
        assert_eq!(out, json!({"authenticated": false, "provider": "github"}));

        tokens
            .store_token(
                PERSON,
                "github",
                "github",
                "tok",
                vec!["repo".into()],
                Some(future()),
                None,
            )
            .await
            .unwrap();
        let out =
            f.call(&r, PERSON, "git.auth_status", json!({"provider":"github"})).await.unwrap();
        assert_eq!(out["authenticated"], true);
        assert_eq!(out["scopes"], json!(["repo"]));
        let expires = out["expires_at"].as_str().unwrap();
        assert!(expires.ends_with("+00:00"), "got {expires}");
        assert!(!expires.contains("tok"));
    }

    #[tokio::test]
    async fn auth_status_reports_all_providers_when_none_is_given() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(
                PERSON,
                "gitlab",
                "gitlab",
                "tok",
                vec!["api".into()],
                Some(future()),
                None,
            )
            .await
            .unwrap();

        let out = f.call(&r, PERSON, "git.auth_status", json!({})).await.unwrap();
        let statuses = out["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0]["provider"], "github");
        assert_eq!(statuses[0]["authenticated"], false);
        assert!(statuses[0].get("scopes").is_none());
        assert_eq!(statuses[1]["provider"], "gitlab");
        assert_eq!(statuses[1]["authenticated"], true);
        assert_eq!(statuses[1]["scopes"], json!(["api"]));
    }

    #[tokio::test]
    async fn an_expired_token_reports_as_unauthenticated() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(
                PERSON,
                "github",
                "github",
                "stale",
                vec![],
                Some(Utc::now() - chrono::Duration::minutes(1)),
                None,
            )
            .await
            .unwrap();
        let out =
            f.call(&r, PERSON, "git.auth_status", json!({"provider":"github"})).await.unwrap();
        assert_eq!(out["authenticated"], false);
    }

    #[tokio::test]
    async fn revoke_clears_the_token_and_is_idempotent() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(PERSON, "github", "github", "tok", vec![], Some(future()), None)
            .await
            .unwrap();

        let out =
            f.call(&r, PERSON, "git.auth_revoke", json!({"provider":"github"})).await.unwrap();
        assert_eq!(out, json!({"provider":"github","revoked":true}));
        assert!(tokens.get_token(PERSON, "github").is_none());

        // revoking again must not fail
        f.call(&r, PERSON, "git.auth_revoke", json!({"provider":"github"})).await.unwrap();
    }

    #[tokio::test]
    async fn tokens_are_per_person() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens
            .store_token(PERSON, "github", "github", "mine", vec![], Some(future()), None)
            .await
            .unwrap();
        let out = f
            .call(&r, "other@test.com", "git.auth_status", json!({"provider":"github"}))
            .await
            .unwrap();
        assert_eq!(out["authenticated"], false, "another person must not inherit a token");
    }

    #[tokio::test]
    async fn an_unauthenticated_caller_is_rejected() {
        let (f, r, _tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        for (name, args) in [
            ("git.auth", json!({"provider":"github"})),
            ("git.auth_status", json!({})),
            ("git.auth_revoke", json!({"provider":"github"})),
        ] {
            let e = f.call(&r, "  ", name, args).await.unwrap_err();
            assert_eq!(e.code, code::UNAUTHENTICATED, "{name}");
        }
    }

    #[test]
    fn round_trip_iso_has_seven_fractional_digits() {
        let dt = DateTime::from_timestamp(1_700_000_000, 123_456_789).unwrap();
        assert_eq!(round_trip_iso(dt), "2023-11-14T22:13:20.1234567+00:00");
        let whole = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        assert_eq!(round_trip_iso(whole), "2023-11-14T22:13:20.0000000+00:00");
    }

    // ── git.token_set (US-004) ───────────────────────────────────────────────────────
    //
    // Every test touching the process wide `git.hosts` map is a plain `#[test]`
    // run through `with_git_hosts_lock`, mirroring `tools/git.rs`'s own
    // convention for the same reason: the map is a `static`, and holding the
    // lock across an `.await` (needed for the whole setup-through-assertions
    // span) is what `with_git_hosts_lock`'s private `block_on` achieves without
    // tripping clippy's `await_holding_lock`.
    //
    // E2E-NEW-022 is a partial, store level test: `resolve_clone_credential`
    // (`crates/mcp-fs/src/tools/git.rs:687`) still resolves a stored token by
    // provider name, not by the real host, because wiring `git.remote_clone`'s
    // credential lookup onto `git.hosts` is US-006's job, and that function is
    // private to `git.rs`, outside this story's two-file scope. This suite
    // proves the store level contract that wiring will read from instead: the
    // exact token `git.token_set` stores for `(person, host)` comes back
    // unmodified, ready to be supplied to the remote as `oauth2:<token>`
    // (`crates/mcp-fs/src/tools/git.rs:1046-1049`).

    const GHP: &str = "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH";

    const REFERENCE_HOSTS: &[(&str, &str)] = &[
        ("github.com", "github"),
        ("github.ibm.com", "github"),
        ("gitlab.acme.corp", "gitlab"),
        ("git.acme.internal", "generic"),
        ("public.example.org", "anonymous"),
    ];

    fn declare_hosts(pairs: &[(&str, &str)]) {
        let mut cfg = crate::config::GitConfig::default();
        cfg.hosts.0 = pairs.iter().map(|(h, p)| (h.to_string(), p.to_string())).collect();
        crate::git::remote::validate_hosts(&cfg).expect("a valid host map");
    }

    fn with_git_hosts_lock<F: std::future::Future>(f: F) -> F::Output {
        let _guard = crate::git::remote::tests::lock_for_test();
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    async fn token_set_registry(tokens: Arc<OAuthTokenStore>) -> (Fixture, ToolRegistry) {
        let f = Fixture::with_config(|c| c.git.enabled = true).await;
        let mut r = ToolRegistry::new();
        register_with(&mut r, Some(tokens), None);
        (f, r)
    }

    /// A `RelationalDb` wrapping a real in-memory SQLite database: `migrate`
    /// and `query` delegate untouched, so opening the store succeeds
    /// normally, but `execute` fails once [`Self::arm`] is called. `upsert` is
    /// the only caller of `execute` on this trait, so arming it fails exactly
    /// the persistence write `store_token` performs.
    struct FailingDb {
        inner: crate::storage::rel::SqliteRelationalDb,
        armed: std::sync::atomic::AtomicBool,
    }

    impl FailingDb {
        fn new() -> Self {
            Self {
                inner: crate::storage::rel::SqliteRelationalDb::open_in_memory().unwrap(),
                armed: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn arm(&self) {
            self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl crate::storage::rel::RelationalDb for FailingDb {
        fn dialect(&self) -> crate::storage::rel::Dialect {
            self.inner.dialect()
        }

        async fn execute(&self, query: &crate::storage::rel::Query) -> Result<u64> {
            if self.armed.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ToolError::internal("failingdb: persistence unavailable"));
            }
            self.inner.execute(query).await
        }

        async fn query(
            &self,
            query: &crate::storage::rel::Query,
        ) -> Result<Vec<crate::storage::rel::RowValues>> {
            self.inner.query(query).await
        }

        async fn begin(&self) -> Result<Box<dyn crate::storage::rel::RelationalTx>> {
            self.inner.begin().await
        }

        async fn migrate(&self, schema: &crate::storage::rel::SchemaSet) -> Result<()> {
            self.inner.migrate(schema).await
        }
    }

    async fn failing_persistent_store() -> (Arc<FailingDb>, Arc<OAuthTokenStore>) {
        let db = Arc::new(FailingDb::new());
        let persistence = Arc::new(
            crate::git::oauth::persistence::RelationalOAuthPersistence::open(
                db.clone(),
                [30u8; crate::git::oauth::cipher::KEY_SIZE],
            )
            .await
            .unwrap(),
        );
        let store = Arc::new(OAuthTokenStore::with_persistence(persistence).await.unwrap());
        (db, store)
    }

    #[tokio::test]
    async fn git_token_set_is_registered_and_takes_no_provider_parameter() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.token_set").unwrap().schema;
        let schema = s.input_schema();
        assert!(schema["properties"].get("provider").is_none());
        assert_eq!(schema["required"], json!(["host", "token"]));
    }

    #[test]
    fn e2e_new_020_seeding_stores_a_token_for_a_declared_host() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            // Persistence configured (unarmed, so it succeeds), matching
            // FR-NEW-013's example output of `"persistent": true` exactly.
            let (_db, tokens) = failing_persistent_store().await;
            let (f, r) = token_set_registry(tokens.clone()).await;

            let out = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();
            assert_eq!(
                out,
                json!({"host":"github.ibm.com","provider":"github","stored":true,"persistent":true})
            );

            let s = tokens.get_token(PERSON, "github.ibm.com").unwrap();
            assert_eq!(s.provider, "github");
            assert_eq!(s.access_token, GHP);
            assert!(tokens.has_valid_token(PERSON, "github.ibm.com"), "validity is 'valid'");
        });
    }

    #[test]
    fn e2e_new_021_seeding_derives_the_provider_from_the_map_not_the_caller() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let schema = r.resolve("git.token_set").unwrap().schema.input_schema();
            assert!(
                schema["properties"].get("provider").is_none(),
                "the tool schema must expose no provider parameter"
            );

            f.call(&r, PERSON, "git.token_set", json!({"host":"git.acme.internal","token":GHP}))
                .await
                .unwrap();
            assert_eq!(tokens.get_token(PERSON, "git.acme.internal").unwrap().provider, "generic");
        });
    }

    #[test]
    fn e2e_new_022_a_seeded_token_is_the_exact_credential_a_clone_would_supply() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let out = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();
            assert_eq!(out["auth".to_string()], Value::Null, "no such key: not this tool's shape");
            assert_eq!(out["provider"], "github");

            let s = tokens.get_token(PERSON, "github.ibm.com").unwrap();
            assert_eq!(
                s.access_token, GHP,
                "the exact bytes a clone would pass as the oauth2 password"
            );
            assert!(s.is_valid_at(Utc::now()));
        });
    }

    #[test]
    fn e2e_new_023_the_response_never_echoes_the_token() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens).await;

            let out = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();
            let serialized = serde_json::to_string(&out).unwrap();
            for start in 0..=(GHP.len() - 8) {
                let chunk = &GHP[start..start + 8];
                assert!(!serialized.contains(chunk), "response must not contain '{chunk}'");
            }
        });
    }

    #[test]
    fn e2e_new_024_an_empty_token_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":""}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
        });
    }

    #[test]
    fn e2e_new_025_a_whitespace_only_token_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(
                    &r,
                    PERSON,
                    "git.token_set",
                    json!({"host":"github.ibm.com","token":"   \t\n "}),
                )
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
        });
    }

    #[test]
    fn e2e_new_026_a_token_over_the_bound_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let long = "a".repeat(8193);
            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":long}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
        });
    }

    #[test]
    fn e2e_new_027_an_undeclared_host_is_rejected_for_seeding() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"git.unknown.test","token":GHP}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("git.unknown.test"), "{}", e.message);
            assert!(tokens.get_token(PERSON, "git.unknown.test").is_none());
        });
    }

    #[test]
    fn e2e_new_028_an_anonymous_host_is_rejected_for_seeding() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"public.example.org","token":GHP}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::INVALID_ARGUMENT);
            assert!(e.message.contains("public.example.org"), "{}", e.message);
            assert!(
                e.message.to_ascii_lowercase().contains("anonymous")
                    && e.message.to_ascii_lowercase().contains("no credential"),
                "the message must state an anonymous host holds no credential: {}",
                e.message
            );
        });
    }

    #[test]
    fn e2e_new_029_an_unauthenticated_caller_is_rejected_for_token_set() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens).await;

            let e = f
                .call(&r, "  ", "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap_err();
            assert_eq!(e.code, code::UNAUTHENTICATED);
        });
    }

    #[test]
    fn e2e_new_030_a_failing_persistence_write_fails_the_call() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (db, tokens) = failing_persistent_store().await;
            assert!(tokens.is_persistent());
            db.arm();
            let (f, r) = token_set_registry(tokens.clone()).await;

            let e = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap_err();
            assert_ne!(e.code, code::INVALID_ARGUMENT, "distinguishable from a validation failure");
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none(), "never reports stored");
        });
    }

    #[test]
    fn e2e_new_031_seeding_twice_overwrites_leaving_one_token() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":"ghp_FIRST0000000000000000000000000"}),
            )
            .await
            .unwrap();
            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":"ghp_SECOND000000000000000000000000"}),
            )
            .await
            .unwrap();

            assert_eq!(
                tokens.list_ids(),
                vec![(PERSON.to_string(), "github.ibm.com".to_string())],
                "exactly one token stored for (person, host)"
            );
            assert_eq!(
                tokens.get_token(PERSON, "github.ibm.com").unwrap().access_token,
                "ghp_SECOND000000000000000000000000"
            );
        });
    }

    #[test]
    fn e2e_new_034_a_token_at_exactly_the_bound_is_accepted() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let exact = "a".repeat(8192);
            f.call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":exact}))
                .await
                .unwrap();
            let stored = tokens.get_token(PERSON, "github.ibm.com").unwrap().access_token;
            assert_eq!(stored.len(), 8192, "stored intact, not truncated");
        });
    }

    #[test]
    fn e2e_new_035_memory_only_operation_is_reported_explicitly() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            assert!(!tokens.is_persistent(), "MCPFS_TOKEN_KEY is unset in this fixture");
            let (f, r) = token_set_registry(tokens).await;

            let out = f
                .call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap();
            assert_eq!(out["persistent"], false);
            let msg = out["message"].as_str().expect("a message explaining memory-only storage");
            assert!(
                msg.to_ascii_lowercase().contains("process")
                    || msg.to_ascii_lowercase().contains("restart"),
                "got {msg}"
            );
        });
    }

    #[test]
    fn e2e_new_191_an_rfc3339_expires_at_is_accepted_in_any_offset() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP,"expires_at":"2027-01-01T00:00:00Z"}),
            )
            .await
            .unwrap();
            let e1 = tokens.get_token(PERSON, "github.ibm.com").unwrap().expires_at.unwrap();

            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({
                    "host":"github.ibm.com","token":GHP,"expires_at":"2027-01-01T01:00:00+01:00"
                }),
            )
            .await
            .unwrap();
            let e2 = tokens.get_token(PERSON, "github.ibm.com").unwrap().expires_at.unwrap();

            assert_eq!(e1, e2, "both offsets must store the same instant");
        });
    }

    #[test]
    fn e2e_new_192_a_non_rfc3339_expires_at_is_rejected() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            for bad in [json!(1_798_761_600i64), json!("tomorrow")] {
                let e = f
                    .call(
                        &r,
                        PERSON,
                        "git.token_set",
                        json!({"host":"github.ibm.com","token":GHP,"expires_at":bad}),
                    )
                    .await
                    .unwrap_err();
                assert_eq!(e.code, code::INVALID_ARGUMENT, "{bad:?}");
                assert!(e.message.contains("expires_at"), "{}", e.message);
            }
            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
        });
    }

    #[test]
    fn e2e_new_193_a_past_expires_at_is_accepted_and_reports_expired() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let tokens = Arc::new(OAuthTokenStore::new());
            let (f, r) = token_set_registry(tokens.clone()).await;

            let past = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":GHP,"expires_at":past}),
            )
            .await
            .unwrap();

            assert!(
                !tokens.has_valid_token(PERSON, "github.ibm.com"),
                "an already-expired token reports invalid"
            );
            assert!(
                tokens.get_token(PERSON, "github.ibm.com").is_some(),
                "the session is still retrievable, only invalid"
            );
        });
    }

    #[test]
    fn e2e_new_221_a_failed_persistence_write_leaves_no_in_memory_token() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (db, tokens) = failing_persistent_store().await;
            db.arm();
            let (f, r) = token_set_registry(tokens.clone()).await;

            f.call(&r, PERSON, "git.token_set", json!({"host":"github.ibm.com","token":GHP}))
                .await
                .unwrap_err();

            assert!(tokens.get_token(PERSON, "github.ibm.com").is_none());
            assert!(!tokens.has_valid_token(PERSON, "github.ibm.com"));
        });
    }

    #[test]
    fn e2e_new_222_a_failed_write_restores_the_previous_token() {
        with_git_hosts_lock(async {
            declare_hosts(REFERENCE_HOSTS);
            let (db, tokens) = failing_persistent_store().await;
            let (f, r) = token_set_registry(tokens.clone()).await;

            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":"ghp_OLD00000000000000000000000000"}),
            )
            .await
            .unwrap();

            db.arm();
            f.call(
                &r,
                PERSON,
                "git.token_set",
                json!({"host":"github.ibm.com","token":"ghp_NEW00000000000000000000000000"}),
            )
            .await
            .unwrap_err();

            let s = tokens.get_token(PERSON, "github.ibm.com").unwrap();
            assert_eq!(s.access_token, "ghp_OLD00000000000000000000000000", "prior token intact");
        });
    }

    /// E2E-NEW-223: the device-flow poller shares the exact same rollback
    /// path as `git.token_set` (both call the one `store_token`), so it needs
    /// no `git.hosts` declaration to prove it: it stores under host = provider
    /// today (see `spawn_poller` above), never touching the host map at all.
    #[tokio::test]
    async fn e2e_new_223_the_device_flow_poller_rolls_back_identically() {
        let (db, tokens) = failing_persistent_store().await;
        db.arm();
        let f = Fixture::with_config(|c| c.git.enabled = true).await;
        let flow =
            FakeFlow::new(vec![TokenPoll::granted("gho_stored", vec!["repo".into()], future())]);
        let mut r = ToolRegistry::new();
        register_with(&mut r, Some(tokens.clone()), Some(flow.clone()));

        f.call(&r, PERSON, "git.auth", json!({"provider":"github"})).await.unwrap();

        assert!(eventually(|| flow.polls() >= 1).await, "the poller must run at least once");
        // Give the poller time to attempt (and fail) the armed persistence write.
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert!(
            tokens.get_token(PERSON, "github").is_none(),
            "no in-memory token must remain after a failed persistence write"
        );
    }
}
