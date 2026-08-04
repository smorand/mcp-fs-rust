//! OAuth device authorization grant (RFC 8628) for GitHub and GitLab.
//!
//! Port of the C# `Git/OAuth/DeviceFlowClient.cs`. Two calls per provider:
//!
//! ```text
//! GitHub  POST https://github.com/login/device/code          -> device_code, user_code
//!         POST https://github.com/login/oauth/access_token    -> access_token (polled)
//! GitLab  POST {instance}/oauth/authorize_device              -> device_code, user_code
//!         POST {instance}/oauth/token                         -> access_token (polled)
//! ```
//!
//! The client id comes from [`GitConfig`]; the client secret is read from the
//! environment variable named by `github_client_secret_env` /
//! `gitlab_client_secret_env` **at call time**, so it is never held in a field,
//! never serialized, and never logged. Both [`DeviceCode`] and [`TokenPoll`]
//! redact their secrets in `Debug`, because a device code is a bearer credential
//! for the pending authorization.
//!
//! [`DeviceFlowClient`] is a trait (the C# `IDeviceFlowClient`) so `git.auth`
//! tests can inject a fake instead of talking to github.com.

use crate::config::GitConfig;
use crate::errors::{Result, ToolError};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

/// Sent on every request, matching the C# client.
pub const USER_AGENT: &str = concat!("mcp-fs/", env!("CARGO_PKG_VERSION"));

pub const GITHUB_DEVICE_URL: &str = "https://github.com/login/device/code";
pub const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// The RFC 8628 grant type, sent when polling for the token.
pub const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Scopes requested per provider, identical to the C#.
const GITHUB_SCOPE: &str = "repo";
const GITLAB_SCOPE: &str = "read_repository write_repository";

/// Fallback poll interval when the provider omits or zeroes `interval`.
const DEFAULT_INTERVAL: i64 = 5;

/// The pending authorization returned by the first call.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceCode {
    /// "github" or "gitlab".
    pub provider: String,
    /// Secret: proves this server started the authorization. Never log it.
    pub device_code: String,
    /// Short code the human types in the browser.
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: i64,
    pub interval: i64,
    /// Self hosted GitLab base URL, `None` for github.com.
    pub instance_url: Option<String>,
}

impl std::fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("provider", &self.provider)
            .field("device_code", &"<redacted>")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .field("instance_url", &self.instance_url)
            .finish()
    }
}

/// One poll result. `success == false` with `error == "authorization_pending"`
/// (or `"slow_down"`) means "keep polling"; `"access_denied"` and
/// `"expired_token"` are terminal.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenPoll {
    pub success: bool,
    pub access_token: Option<String>,
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub error: Option<String>,
}

impl TokenPoll {
    pub fn granted(
        access_token: impl Into<String>,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            success: true,
            access_token: Some(access_token.into()),
            scopes,
            expires_at,
            error: None,
        }
    }

    pub fn pending(error: impl Into<String>) -> Self {
        Self {
            success: false,
            access_token: None,
            scopes: Vec::new(),
            expires_at: DateTime::UNIX_EPOCH,
            error: Some(error.into()),
        }
    }

    /// True when polling must stop: the user refused, or the code died.
    pub fn is_terminal_error(&self) -> bool {
        matches!(self.error.as_deref(), Some("access_denied") | Some("expired_token"))
    }
}

impl std::fmt::Debug for TokenPoll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenPoll")
            .field("success", &self.success)
            .field("access_token", &self.access_token.as_ref().map(|_| "<redacted>"))
            .field("scopes", &self.scopes)
            .field("expires_at", &self.expires_at)
            .field("error", &self.error)
            .finish()
    }
}

/// Device flow transport. The C# `IDeviceFlowClient`: a trait so tests inject a
/// fake and never reach the network.
#[async_trait]
pub trait DeviceFlowClient: Send + Sync {
    /// Start an authorization. `instance_url` overrides the configured GitLab base.
    async fn request_device_code(
        &self,
        provider: &str,
        instance_url: Option<&str>,
    ) -> Result<DeviceCode>;

    /// One poll of the token endpoint.
    async fn poll_for_token(&self, code: &DeviceCode) -> Result<TokenPoll>;
}

/// Real HTTP implementation.
pub struct HttpDeviceFlowClient {
    http: reqwest::Client,
    config: GitConfig,
    github_device_url: String,
    github_token_url: String,
}

impl HttpDeviceFlowClient {
    pub fn new(config: GitConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| ToolError::internal(format!("http client build failed: {e}")))?;
        Ok(Self {
            http,
            config,
            github_device_url: GITHUB_DEVICE_URL.to_string(),
            github_token_url: GITHUB_TOKEN_URL.to_string(),
        })
    }

    /// Point the GitHub endpoints somewhere else, so the parsing and the form
    /// encoding can be tested against a local server instead of github.com.
    pub fn with_github_urls(mut self, device_url: impl Into<String>, token_url: impl Into<String>) -> Self {
        self.github_device_url = device_url.into();
        self.github_token_url = token_url.into();
        self
    }

    /// Read a client secret from the environment at call time. Returning an error
    /// (instead of posting an empty secret) keeps the failure legible.
    fn secret(env_name: &str, provider: &str) -> Result<String> {
        match std::env::var(env_name) {
            Ok(v) if !v.trim().is_empty() => Ok(v),
            _ => Err(ToolError::invalid_argument(format!(
                "{provider} client secret not set. Set ${env_name} environment variable."
            ))),
        }
    }

    fn gitlab_base(&self, instance_url: Option<&str>) -> String {
        let raw = instance_url
            .filter(|u| !u.trim().is_empty())
            .unwrap_or(&self.config.gitlab_instance_url);
        raw.trim_end_matches('/').to_string()
    }

    async fn post_form<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        form: &[(&str, &str)],
        what: &str,
    ) -> Result<T> {
        let resp = self
            .http
            .post(url)
            // GitHub answers form encoded unless asked for JSON.
            .header(reqwest::header::ACCEPT, "application/json")
            .form(form)
            .send()
            .await
            .map_err(|e| ToolError::internal(format!("{what} request failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ToolError::internal(format!("{what} returned HTTP {}", status.as_u16())));
        }
        resp.json::<T>()
            .await
            .map_err(|e| ToolError::internal(format!("{what} returned an unreadable body: {e}")))
    }

    async fn github_device_code(&self) -> Result<DeviceCode> {
        let client_id = self.config.github_client_id.trim();
        if client_id.is_empty() {
            return Err(ToolError::invalid_argument(
                "git.github_client_id is not configured. See .agent_docs/github-oauth-setup.md",
            ));
        }
        let json: DeviceCodeBody = self
            .post_form(
                &self.github_device_url,
                &[("client_id", client_id), ("scope", GITHUB_SCOPE)],
                "GitHub device/code",
            )
            .await?;
        Ok(DeviceCode {
            provider: "github".into(),
            device_code: json.device_code,
            user_code: json.user_code,
            verification_uri: json.verification_uri,
            expires_in: json.expires_in,
            interval: if json.interval > 0 { json.interval } else { DEFAULT_INTERVAL },
            instance_url: None,
        })
    }

    async fn gitlab_device_code(&self, instance_url: Option<&str>) -> Result<DeviceCode> {
        let client_id = self.config.gitlab_client_id.trim();
        if client_id.is_empty() {
            return Err(ToolError::invalid_argument(
                "git.gitlab_client_id is not configured. See .agent_docs/github-oauth-setup.md",
            ));
        }
        let base = self.gitlab_base(instance_url);
        let json: DeviceCodeBody = self
            .post_form(
                &format!("{base}/oauth/authorize_device"),
                &[("client_id", client_id), ("scope", GITLAB_SCOPE)],
                "GitLab authorize_device",
            )
            .await?;
        Ok(DeviceCode {
            provider: "gitlab".into(),
            device_code: json.device_code,
            // GitLab offers a prefilled URI; prefer it, the user then types nothing.
            verification_uri: json
                .verification_uri_complete
                .filter(|u| !u.is_empty())
                .unwrap_or(json.verification_uri),
            user_code: json.user_code,
            expires_in: json.expires_in,
            interval: if json.interval > 0 { json.interval } else { DEFAULT_INTERVAL },
            instance_url: Some(base),
        })
    }

    async fn github_poll(&self, code: &DeviceCode) -> Result<TokenPoll> {
        let secret = Self::secret(&self.config.github_client_secret_env, "GitHub")?;
        let json: TokenBody = self
            .post_form(
                &self.github_token_url,
                &[
                    ("client_id", self.config.github_client_id.trim()),
                    ("client_secret", &secret),
                    ("device_code", &code.device_code),
                    ("grant_type", DEVICE_GRANT_TYPE),
                ],
                "GitHub access_token",
            )
            .await?;
        // GitHub returns comma separated scopes and, by default, tokens that never
        // expire; 8 hours keeps a bounded session like the C#.
        Ok(json.into_poll(',', 8))
    }

    async fn gitlab_poll(&self, code: &DeviceCode) -> Result<TokenPoll> {
        let secret = Self::secret(&self.config.gitlab_client_secret_env, "GitLab")?;
        let base = self.gitlab_base(code.instance_url.as_deref());
        let json: TokenBody = self
            .post_form(
                &format!("{base}/oauth/token"),
                &[
                    ("client_id", self.config.gitlab_client_id.trim()),
                    ("client_secret", &secret),
                    ("device_code", &code.device_code),
                    ("grant_type", DEVICE_GRANT_TYPE),
                ],
                "GitLab token",
            )
            .await?;
        // GitLab returns space separated scopes and short lived tokens.
        Ok(json.into_poll(' ', 2))
    }
}

#[async_trait]
impl DeviceFlowClient for HttpDeviceFlowClient {
    async fn request_device_code(
        &self,
        provider: &str,
        instance_url: Option<&str>,
    ) -> Result<DeviceCode> {
        match provider.to_ascii_lowercase().as_str() {
            "github" => self.github_device_code().await,
            "gitlab" => self.gitlab_device_code(instance_url).await,
            other => Err(ToolError::invalid_argument(format!(
                "unknown OAuth provider: {other}. Supported: github, gitlab."
            ))),
        }
    }

    async fn poll_for_token(&self, code: &DeviceCode) -> Result<TokenPoll> {
        match code.provider.to_ascii_lowercase().as_str() {
            "github" => self.github_poll(code).await,
            "gitlab" => self.gitlab_poll(code).await,
            other => Err(ToolError::invalid_argument(format!(
                "unknown OAuth provider: {other}. Supported: github, gitlab."
            ))),
        }
    }
}

// ── wire bodies ─────────────────────────────────────────────────────────────

/// Both providers answer the device endpoint with the same RFC 8628 fields;
/// `verification_uri_complete` is a GitLab extension.
#[derive(Debug, Deserialize)]
struct DeviceCodeBody {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    user_code: String,
    #[serde(default)]
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    interval: i64,
}

#[derive(Debug, Deserialize)]
struct TokenBody {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    error: Option<String>,
}

impl TokenBody {
    /// `separator` splits the scope string, `default_hours` bounds a token whose
    /// response carries no `expires_in`.
    fn into_poll(self, separator: char, default_hours: i64) -> TokenPoll {
        match self.access_token.filter(|t| !t.is_empty()) {
            Some(token) => {
                let scopes = self
                    .scope
                    .unwrap_or_default()
                    .split(separator)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                let expires_at = if self.expires_in > 0 {
                    Utc::now() + Duration::seconds(self.expires_in)
                } else {
                    Utc::now() + Duration::hours(default_hours)
                };
                TokenPoll::granted(token, scopes, expires_at)
            }
            // No token yet: the provider tells us why, defaulting to "keep polling".
            None => TokenPoll::pending(self.error.unwrap_or_else(|| "authorization_pending".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::Form;
    use axum::routing::post;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Each test owns its own secret variable names: environment variables are
    /// process wide and the test binary runs tests in parallel, so sharing one
    /// name would make the suite order dependent.
    fn config_named(gh_env: &str, gl_env: &str) -> GitConfig {
        GitConfig {
            enabled: true,
            github_client_id: "gh-client".into(),
            github_client_secret_env: gh_env.into(),
            gitlab_client_id: "gl-client".into(),
            gitlab_client_secret_env: gl_env.into(),
            ..Default::default()
        }
    }

    /// Config whose secret variables are never set by any test.
    fn config() -> GitConfig {
        config_named("MCPFS_TEST_GH_SECRET_UNSET", "MCPFS_TEST_GL_SECRET_UNSET")
    }

    /// Records the forms a provider received, so the request can be asserted on.
    type Seen = Arc<Mutex<Vec<HashMap<String, String>>>>;

    /// A local stand-in for github.com / gitlab.com. Returns `device_body` from the
    /// device endpoints and pops `token_bodies` on each poll.
    async fn provider_server(
        device_body: serde_json::Value,
        token_bodies: Vec<serde_json::Value>,
    ) -> (String, Seen) {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::new(Mutex::new(token_bodies));

        let d_seen = seen.clone();
        let t_seen = seen.clone();
        let device = move |Form(f): Form<HashMap<String, String>>| {
            let seen = d_seen.clone();
            let body = device_body.clone();
            async move {
                seen.lock().unwrap().push(f);
                axum::Json(body)
            }
        };
        let token = move |Form(f): Form<HashMap<String, String>>| {
            let seen = t_seen.clone();
            let queue = queue.clone();
            async move {
                seen.lock().unwrap().push(f);
                let mut q = queue.lock().unwrap();
                let body = if q.len() > 1 { q.remove(0) } else { q[0].clone() };
                axum::Json(body)
            }
        };

        let app = Router::new()
            .route("/login/device/code", post(device.clone()))
            .route("/oauth/authorize_device", post(device))
            .route("/login/oauth/access_token", post(token.clone()))
            .route("/oauth/token", post(token));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), seen)
    }

    fn github_client(base: &str) -> HttpDeviceFlowClient {
        github_client_with(base, config())
    }

    fn github_client_with(base: &str, config: GitConfig) -> HttpDeviceFlowClient {
        HttpDeviceFlowClient::new(config).unwrap().with_github_urls(
            format!("{base}/login/device/code"),
            format!("{base}/login/oauth/access_token"),
        )
    }

    #[tokio::test]
    async fn github_device_code_is_parsed_and_the_form_is_correct() {
        let (base, seen) = provider_server(
            serde_json::json!({
                "device_code": "dc-secret", "user_code": "ABCD-1234",
                "verification_uri": "https://github.com/login/device",
                "expires_in": 900, "interval": 7
            }),
            vec![serde_json::json!({})],
        )
        .await;

        let c = github_client(&base);
        let code = c.request_device_code("GitHub", None).await.unwrap();
        assert_eq!(code.provider, "github");
        assert_eq!(code.device_code, "dc-secret");
        assert_eq!(code.user_code, "ABCD-1234");
        assert_eq!(code.verification_uri, "https://github.com/login/device");
        assert_eq!(code.expires_in, 900);
        assert_eq!(code.interval, 7);
        assert!(code.instance_url.is_none());

        let forms = seen.lock().unwrap();
        assert_eq!(forms[0]["client_id"], "gh-client");
        assert_eq!(forms[0]["scope"], "repo");
    }

    #[tokio::test]
    async fn missing_interval_falls_back_to_five_seconds() {
        let (base, _) = provider_server(
            serde_json::json!({"device_code": "d", "user_code": "u", "verification_uri": "v",
                               "expires_in": 600}),
            vec![serde_json::json!({})],
        )
        .await;
        let code = github_client(&base).request_device_code("github", None).await.unwrap();
        assert_eq!(code.interval, DEFAULT_INTERVAL);
    }

    #[tokio::test]
    async fn github_poll_success_sends_the_secret_from_the_environment() {
        const ENV: &str = "MCPFS_TEST_GH_SECRET_POLL_OK";
        // SAFETY: this test owns this variable name, no other test touches it.
        unsafe { std::env::set_var(ENV, "gh-secret") };
        let (base, seen) = provider_server(
            serde_json::json!({"device_code": "dc", "user_code": "u", "verification_uri": "v",
                               "expires_in": 600, "interval": 1}),
            vec![serde_json::json!({"access_token": "gho_tok", "scope": "repo,read:user"})],
        )
        .await;
        let c = github_client_with(&base, config_named(ENV, "unused"));
        let code = c.request_device_code("github", None).await.unwrap();
        let poll = c.poll_for_token(&code).await.unwrap();

        assert!(poll.success);
        assert_eq!(poll.access_token.as_deref(), Some("gho_tok"));
        assert_eq!(poll.scopes, vec!["repo", "read:user"]);
        // no expires_in in the response: the 8 hour default applies
        let ttl = poll.expires_at - Utc::now();
        assert!(ttl > Duration::hours(7) && ttl <= Duration::hours(8), "got {ttl}");

        let forms = seen.lock().unwrap();
        let token_form = forms.last().unwrap();
        assert_eq!(token_form["client_id"], "gh-client");
        assert_eq!(token_form["client_secret"], "gh-secret");
        assert_eq!(token_form["device_code"], "dc");
        assert_eq!(token_form["grant_type"], DEVICE_GRANT_TYPE);
        unsafe { std::env::remove_var(ENV) };
    }

    #[tokio::test]
    async fn github_poll_honours_expires_in() {
        const ENV: &str = "MCPFS_TEST_GH_SECRET_EXPIRY";
        unsafe { std::env::set_var(ENV, "s") };
        let (base, _) = provider_server(
            serde_json::json!({"device_code": "dc", "user_code": "u", "verification_uri": "v",
                               "expires_in": 600, "interval": 1}),
            vec![serde_json::json!({"access_token": "t", "scope": "repo", "expires_in": 120})],
        )
        .await;
        let c = github_client_with(&base, config_named(ENV, "unused"));
        let code = c.request_device_code("github", None).await.unwrap();
        let poll = c.poll_for_token(&code).await.unwrap();
        let ttl = poll.expires_at - Utc::now();
        assert!(ttl > Duration::seconds(100) && ttl <= Duration::seconds(120), "got {ttl}");
        unsafe { std::env::remove_var(ENV) };
    }

    #[tokio::test]
    async fn pending_and_denied_are_distinguished() {
        const ENV: &str = "MCPFS_TEST_GH_SECRET_PENDING";
        unsafe { std::env::set_var(ENV, "s") };
        let (base, _) = provider_server(
            serde_json::json!({"device_code": "dc", "user_code": "u", "verification_uri": "v",
                               "expires_in": 600, "interval": 1}),
            vec![
                serde_json::json!({"error": "authorization_pending"}),
                serde_json::json!({"error": "access_denied"}),
            ],
        )
        .await;
        let c = github_client_with(&base, config_named(ENV, "unused"));
        let code = c.request_device_code("github", None).await.unwrap();

        let first = c.poll_for_token(&code).await.unwrap();
        assert!(!first.success);
        assert_eq!(first.error.as_deref(), Some("authorization_pending"));
        assert!(!first.is_terminal_error(), "pending must keep the loop alive");

        let second = c.poll_for_token(&code).await.unwrap();
        assert_eq!(second.error.as_deref(), Some("access_denied"));
        assert!(second.is_terminal_error(), "a refusal must stop the loop");
        unsafe { std::env::remove_var(ENV) };
    }

    #[tokio::test]
    async fn a_body_with_neither_token_nor_error_is_treated_as_pending() {
        const ENV: &str = "MCPFS_TEST_GH_SECRET_NEITHER";
        unsafe { std::env::set_var(ENV, "s") };
        let (base, _) = provider_server(
            serde_json::json!({"device_code": "dc", "user_code": "u", "verification_uri": "v",
                               "expires_in": 60, "interval": 1}),
            vec![serde_json::json!({})],
        )
        .await;
        let c = github_client_with(&base, config_named(ENV, "unused"));
        let code = c.request_device_code("github", None).await.unwrap();
        let poll = c.poll_for_token(&code).await.unwrap();
        assert_eq!(poll.error.as_deref(), Some("authorization_pending"));
        unsafe { std::env::remove_var(ENV) };
    }

    #[tokio::test]
    async fn gitlab_uses_the_instance_url_and_prefers_the_complete_uri() {
        const ENV: &str = "MCPFS_TEST_GL_SECRET_INSTANCE";
        unsafe { std::env::set_var(ENV, "gl-secret") };
        let (base, seen) = provider_server(
            serde_json::json!({
                "device_code": "gl-dc", "user_code": "XYZ",
                "verification_uri": "https://gl.test/oauth/device",
                "verification_uri_complete": "https://gl.test/oauth/device?user_code=XYZ",
                "expires_in": 300, "interval": 2
            }),
            vec![serde_json::json!({"access_token": "glpat", "scope": "read_repository write_repository"})],
        )
        .await;

        let c = HttpDeviceFlowClient::new(config_named("unused", ENV)).unwrap();
        // a trailing slash must not double up in the built URL
        let instance = format!("{base}/");
        let code = c.request_device_code("gitlab", Some(&instance)).await.unwrap();
        assert_eq!(code.provider, "gitlab");
        assert_eq!(code.verification_uri, "https://gl.test/oauth/device?user_code=XYZ");
        assert_eq!(code.instance_url.as_deref(), Some(base.as_str()));

        let poll = c.poll_for_token(&code).await.unwrap();
        assert!(poll.success);
        assert_eq!(poll.scopes, vec!["read_repository", "write_repository"]);
        let ttl = poll.expires_at - Utc::now();
        assert!(ttl <= Duration::hours(2), "gitlab defaults to a 2 hour session");

        let forms = seen.lock().unwrap();
        assert_eq!(forms[0]["scope"], GITLAB_SCOPE);
        assert_eq!(forms.last().unwrap()["client_secret"], "gl-secret");
        unsafe { std::env::remove_var(ENV) };
    }

    #[tokio::test]
    async fn an_unconfigured_client_id_is_an_invalid_argument() {
        let mut cfg = config();
        cfg.github_client_id = "  ".into();
        cfg.gitlab_client_id = String::new();
        let c = HttpDeviceFlowClient::new(cfg).unwrap();

        let e = c.request_device_code("github", None).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("github_client_id"));

        let e = c.request_device_code("gitlab", None).await.unwrap_err();
        assert!(e.message.contains("gitlab_client_id"));
    }

    #[tokio::test]
    async fn a_missing_client_secret_is_reported_not_sent_empty() {
        // The variable name in `config()` is never set by any test.
        let c = HttpDeviceFlowClient::new(config()).unwrap();
        let code = DeviceCode {
            provider: "github".into(),
            device_code: "dc".into(),
            user_code: "u".into(),
            verification_uri: "v".into(),
            expires_in: 60,
            interval: 5,
            instance_url: None,
        };
        let e = c.poll_for_token(&code).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("MCPFS_TEST_GH_SECRET_UNSET"), "got {}", e.message);
    }

    #[tokio::test]
    async fn unknown_providers_are_rejected_on_both_calls() {
        let c = HttpDeviceFlowClient::new(config()).unwrap();
        let e = c.request_device_code("bitbucket", None).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("Supported: github, gitlab."));

        let code = DeviceCode {
            provider: "bitbucket".into(),
            device_code: "d".into(),
            user_code: "u".into(),
            verification_uri: "v".into(),
            expires_in: 1,
            interval: 1,
            instance_url: None,
        };
        assert!(c.poll_for_token(&code).await.is_err());
    }

    #[tokio::test]
    async fn a_provider_http_error_is_surfaced() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/login/device/code",
            post(|| async { (axum::http::StatusCode::SERVICE_UNAVAILABLE, "down") }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let c = github_client(&format!("http://{addr}"));
        let e = c.request_device_code("github", None).await.unwrap_err();
        assert_eq!(e.code, crate::errors::code::INTERNAL_ERROR);
        assert!(e.message.contains("HTTP 503"), "got {}", e.message);
    }

    #[test]
    fn debug_output_redacts_both_secrets() {
        let code = DeviceCode {
            provider: "github".into(),
            device_code: "dc-verysecret".into(),
            user_code: "ABCD".into(),
            verification_uri: "v".into(),
            expires_in: 1,
            interval: 1,
            instance_url: None,
        };
        let s = format!("{code:?}");
        assert!(!s.contains("dc-verysecret"), "the device code is a credential");
        assert!(s.contains("ABCD"), "the user code is meant to be shown");

        let poll = TokenPoll::granted("gho_secret", vec!["repo".into()], Utc::now());
        let s = format!("{poll:?}");
        assert!(!s.contains("gho_secret"));
        assert!(s.contains("<redacted>"));
    }
}
