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

/// A provider that returns `interval: 0` must not turn the poll loop into a spin.
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Process wide token store, built once from the config. `git.auth` writes it and
/// `git.remote_clone` reads it, so they must be the same instance; the error is
/// cached too, because a malformed `MCPFS_TOKEN_KEY` must fail loudly every time
/// instead of silently downgrading to memory only storage.
static TOKENS: OnceLock<Result<Arc<OAuthTokenStore>>> = OnceLock::new();

/// Process wide device flow client. Reuses one `reqwest` connection pool.
static FLOW: OnceLock<Result<Arc<dyn DeviceFlowClient>>> = OnceLock::new();

pub fn token_store(config: &ServerConfig) -> Result<Arc<OAuthTokenStore>> {
    TOKENS.get_or_init(|| OAuthTokenStore::from_env(config).map(Arc::new)).clone()
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
        .opt_str_null("instance_url", "Optional self-hosted instance URL (e.g. GitLab Enterprise)."),
        handler(move |ctx: ToolCtx, a| {
            let (t, f) = (t.clone(), f.clone());
            async move {
                let provider = a.str("provider")?;
                let instance_url = a.opt_str("instance_url");
                let tokens = resolve_tokens(&ctx, t)?;
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
        .opt_str_null("provider", "Provider to check: github or gitlab; omit to report all providers."),
        handler(move |ctx: ToolCtx, a| {
            let t = t.clone();
            async move {
                let tokens = resolve_tokens(&ctx, t)?;
                auth_status(&ctx, a.opt_str("provider").as_deref(), &tokens)
            }
        }),
    );

    let t = tokens;
    reg.add(
        ToolSchema::new("git.auth_revoke", "Revoke the stored token for a provider.")
            .req_str("provider", "Provider whose stored token is revoked: github or gitlab."),
        handler(move |ctx: ToolCtx, a| {
            let t = t.clone();
            async move {
                let provider = a.str("provider")?;
                let tokens = resolve_tokens(&ctx, t)?;
                auth_revoke(&ctx, &provider, &tokens)
            }
        }),
    );
}

fn resolve_tokens(ctx: &ToolCtx, injected: Option<Arc<OAuthTokenStore>>) -> Result<Arc<OAuthTokenStore>> {
    match injected {
        Some(t) => Ok(t),
        None => token_store(&ctx.state.config),
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

    let message =
        format!("Open {} and enter code {}", code.verification_uri, code.user_code);
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
    let interval =
        Duration::from_secs(code.interval.max(0) as u64).max(MIN_POLL_INTERVAL);
    let lifetime = Duration::from_secs(code.expires_in.max(0) as u64);
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + lifetime;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(interval).await;
            match flow.poll_for_token(&code).await {
                Ok(poll) if poll.success => {
                    let Some(token) = poll.access_token else { return };
                    if let Err(e) = tokens.store_token(
                        &person,
                        &provider,
                        &token,
                        poll.scopes,
                        poll.expires_at,
                        instance_url,
                    ) {
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
    if authenticated
        && let Some(s) = tokens.get_token(person, provider)
    {
        out.insert("scopes".into(), json!(s.scopes));
        out.insert("expires_at".into(), json!(round_trip_iso(s.expires_at)));
    }
    Value::Object(out)
}

fn auth_revoke(ctx: &ToolCtx, provider: &str, tokens: &OAuthTokenStore) -> Result<Value> {
    let person = require_identity(ctx)?;
    tokens.revoke_token(&person, provider)?;
    Ok(json!({"provider": provider, "revoked": true}))
}

/// The C# `DateTimeOffset.ToString("O")`: seven fractional digits (100ns ticks)
/// plus an explicit offset, which is always UTC here.
fn round_trip_iso(dt: DateTime<Utc>) -> String {
    format!(
        "{}{:07}+00:00",
        dt.format("%Y-%m-%dT%H:%M:%S."),
        dt.timestamp_subsec_nanos() / 100
    )
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
    async fn setup(script: Vec<TokenPoll>) -> (Fixture, ToolRegistry, Arc<OAuthTokenStore>, Arc<FakeFlow>) {
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
        assert_eq!(r.len(), 3);
        for name in ["git.auth", "git.auth_status", "git.auth_revoke"] {
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
        assert_eq!(
            out["message"],
            "Open https://github.com/login/device and enter code WXYZ-9876"
        );
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
        let out = f
            .call(&r, PERSON, "git.auth_status", json!({"provider":"github"}))
            .await
            .unwrap();
        assert_eq!(out, json!({"authenticated": false, "provider": "github"}));

        tokens
            .store_token(PERSON, "github", "tok", vec!["repo".into()], future(), None)
            .unwrap();
        let out = f
            .call(&r, PERSON, "git.auth_status", json!({"provider":"github"}))
            .await
            .unwrap();
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
            .store_token(PERSON, "gitlab", "tok", vec!["api".into()], future(), None)
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
                "stale",
                vec![],
                Utc::now() - chrono::Duration::minutes(1),
                None,
            )
            .unwrap();
        let out = f
            .call(&r, PERSON, "git.auth_status", json!({"provider":"github"}))
            .await
            .unwrap();
        assert_eq!(out["authenticated"], false);
    }

    #[tokio::test]
    async fn revoke_clears_the_token_and_is_idempotent() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens.store_token(PERSON, "github", "tok", vec![], future(), None).unwrap();

        let out = f
            .call(&r, PERSON, "git.auth_revoke", json!({"provider":"github"}))
            .await
            .unwrap();
        assert_eq!(out, json!({"provider":"github","revoked":true}));
        assert!(tokens.get_token(PERSON, "github").is_none());

        // revoking again must not fail
        f.call(&r, PERSON, "git.auth_revoke", json!({"provider":"github"})).await.unwrap();
    }

    #[tokio::test]
    async fn tokens_are_per_person() {
        let (f, r, tokens, _flow) = setup(vec![TokenPoll::pending("x")]).await;
        tokens.store_token(PERSON, "github", "mine", vec![], future(), None).unwrap();
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
}
