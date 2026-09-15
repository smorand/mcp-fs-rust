//! Server configuration. Mirrors the C# `Models/Config.cs` schema key for key,
//! default for default, so the same YAML file drives both implementations.
//!
//! `${VAR}` and `${VAR:-default}` are expanded from the environment in the raw
//! YAML **before** parsing, so secrets never live in committed config files.

use crate::errors::{Result, ToolError};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

/// The relational backends a store section may name.
pub mod backend {
    pub const SQLITE: &str = "sqlite";
    pub const POSTGRES: &str = "postgres";
    pub const SQLSERVER: &str = "sqlserver";

    /// Every accepted value, for error messages.
    pub const ALL: [&str; 3] = [SQLITE, POSTGRES, SQLSERVER];
}

/// A database connection string.
///
/// A DSN carries a password, so this newtype refuses to print itself: `Debug` and
/// `Display` both redact, which is what keeps it out of a log line, a boot banner
/// and an error message. [`Self::expose`] is the single audited read point.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Dsn(String);

impl Dsn {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// An unset DSN. A store section with a SQLite backend leaves it empty.
    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }

    /// The real connection string, for handing to a driver and nothing else.
    pub fn expose(&self) -> &str {
        self.0.trim()
    }
}

/// Deliberately lossy. Reporting whether a DSN is set is useful when diagnosing a
/// boot failure; reporting its contents would publish a password.
impl fmt::Debug for Dsn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() { f.write_str("Dsn(unset)") } else { f.write_str("Dsn(redacted)") }
    }
}

impl fmt::Display for Dsn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() { f.write_str("unset") } else { f.write_str("redacted") }
    }
}

fn d_pool_max_connections() -> u32 { 10 }
fn d_pool_acquire_timeout() -> u64 { 30 }
fn d_pg_schema() -> String { "public".into() }

/// Connection pool sizing. Ignored by the SQLite backend, which keeps one
/// serialized connection by design.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PoolConfig {
    pub max_connections: u32,
    pub acquire_timeout_secs: u64,
}
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: d_pool_max_connections(),
            acquire_timeout_secs: d_pool_acquire_timeout(),
        }
    }
}

fn d_host() -> String { "0.0.0.0".into() }
fn d_port() -> u16 { 5002 }
fn d_mcp_path() -> String { "/mcp".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
    pub mcp_path: String,
}
impl Default for HttpConfig {
    fn default() -> Self {
        Self { host: d_host(), port: d_port(), mcp_path: d_mcp_path() }
    }
}

fn d_header() -> String { "X-Forwarded-Authorization".into() }
fn d_algorithms() -> Vec<String> { vec!["RS256".into()] }
fn d_issuer() -> Option<String> { Some("web-a2a".into()) }
fn d_username_claim() -> String { "email".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JwtConfig {
    pub public_key_path: String,
    pub header: String,
    pub algorithms: Vec<String>,
    pub audience: Option<String>,
    pub issuer: Option<String>,
    pub username_claim: String,
}
impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            public_key_path: String::new(),
            header: d_header(),
            algorithms: d_algorithms(),
            audience: None,
            issuer: d_issuer(),
            username_claim: d_username_claim(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub jwt: JwtConfig,
    /// Platform admins (caseless match).
    pub admins: Vec<String>,
}

fn d_meta_backend() -> String { "sqlite".into() }
fn d_meta_dir() -> String { "state/volumes".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetaConfig {
    pub backend: String,
    /// SQLite only: one database file per volume under this directory.
    pub dir: String,
    /// PostgreSQL and SQL Server only. Secret, so inject it with `${VAR}`.
    pub dsn: Dsn,
    /// PostgreSQL only: the schema holding our tables.
    pub schema: String,
    pub pool: PoolConfig,
}
impl Default for MetaConfig {
    fn default() -> Self {
        Self {
            backend: d_meta_backend(),
            dir: d_meta_dir(),
            dsn: Dsn::default(),
            schema: d_pg_schema(),
            pool: PoolConfig::default(),
        }
    }
}

fn d_blob_backend() -> String { "local".into() }
fn d_blob_dir() -> String { "state/blobs".into() }
fn d_bucket_prefix() -> String { "mcpfs-".into() }
fn d_region() -> String { "us-east-1".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BlobConfig {
    /// "local" (filesystem, default) or "minio"/"s3".
    pub backend: String,
    pub dir: String,
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket_prefix: String,
    pub region: String,
}
impl Default for BlobConfig {
    fn default() -> Self {
        Self {
            backend: d_blob_backend(),
            dir: d_blob_dir(),
            endpoint: String::new(),
            access_key: String::new(),
            secret_key: String::new(),
            bucket_prefix: d_bucket_prefix(),
            region: d_region(),
        }
    }
}

fn d_admin_backend() -> String { "sqlite".into() }
fn d_admin_path() -> String { "state/admin.db".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AdminConfig {
    pub backend: String,
    /// SQLite only: the ACL registry file.
    pub path: String,
    pub dsn: Dsn,
    pub schema: String,
    pub pool: PoolConfig,
}
impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            backend: d_admin_backend(),
            path: d_admin_path(),
            dsn: Dsn::default(),
            schema: d_pg_schema(),
            pool: PoolConfig::default(),
        }
    }
}

/// The git index store (`git_objects`, `git_refs`, `git_remotes`).
///
/// New section: this store existed before but its location was hardcoded, so it
/// could not be pointed at another engine. Absent, it keeps the derived SQLite
/// path `{state_root}/git/{project_id}.db`, so no existing deployment changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GitStoreConfig {
    pub backend: String,
    pub dsn: Dsn,
    pub schema: String,
    pub pool: PoolConfig,
}
impl Default for GitStoreConfig {
    fn default() -> Self {
        Self {
            backend: backend::SQLITE.into(),
            dsn: Dsn::default(),
            schema: d_pg_schema(),
            pool: PoolConfig::default(),
        }
    }
}

/// The OAuth token store. Same story as [`GitStoreConfig`]: previously hardcoded
/// to `{state_root}/oauth.db`, which stays the default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OauthStoreConfig {
    pub backend: String,
    pub dsn: Dsn,
    pub schema: String,
    pub pool: PoolConfig,
}
impl Default for OauthStoreConfig {
    fn default() -> Self {
        Self {
            backend: backend::SQLITE.into(),
            dsn: Dsn::default(),
            schema: d_pg_schema(),
            pool: PoolConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InfraConfig {
    pub meta: MetaConfig,
    pub blob: BlobConfig,
    pub admin: AdminConfig,
    pub git: GitStoreConfig,
    pub oauth: OauthStoreConfig,
}

fn d_write_quota() -> i64 { 50 * 1024 * 1024 }
fn d_trash_dir() -> String { ".mcp_trash".into() }
fn d_max_read_lines() -> usize { 2000 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetyConfig {
    pub write_quota_bytes: i64,
    pub trash_dir: String,
    pub read_guard: bool,
    pub allow_hard_delete: bool,
    pub max_read_lines: usize,
}
impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            write_quota_bytes: d_write_quota(),
            trash_dir: d_trash_dir(),
            read_guard: true,
            allow_hard_delete: false,
            max_read_lines: d_max_read_lines(),
        }
    }
}

fn d_ocr_provider() -> String { "none".into() }
fn d_ocr_key_env() -> String { "MCP_FS_OCR_KEY".into() }
fn d_ocr_prompt() -> String {
    "Transcribe this document faithfully into Markdown.".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OcrConfig {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub api_key_env: String,
    pub prompt: String,
}
impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            provider: d_ocr_provider(),
            endpoint: String::new(),
            model: String::new(),
            api_key_env: d_ocr_key_env(),
            prompt: d_ocr_prompt(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractConfig {
    pub ocr: OcrConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    pub enabled: bool,
}
impl Default for ApiConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn d_object_format() -> String { "sha1".into() }
fn d_max_pack_mb() -> u32 { 512 }
fn d_github_secret_env() -> String { "MCPFS_GITHUB_CLIENT_SECRET".into() }
fn d_gitlab_secret_env() -> String { "GITLAB_CLIENT_SECRET".into() }
fn d_gitlab_url() -> String { "https://gitlab.com".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GitConfig {
    pub enabled: bool,
    pub object_format: String,
    pub anonymous_read: bool,
    pub max_pack_size_mb: u32,
    pub github_client_id: String,
    pub github_client_secret_env: String,
    pub gitlab_client_id: String,
    pub gitlab_client_secret_env: String,
    pub gitlab_instance_url: String,
}
impl Default for GitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            object_format: d_object_format(),
            anonymous_read: false,
            max_pack_size_mb: d_max_pack_mb(),
            github_client_id: String::new(),
            github_client_secret_env: d_github_secret_env(),
            gitlab_client_id: String::new(),
            gitlab_client_secret_env: d_gitlab_secret_env(),
            gitlab_instance_url: d_gitlab_url(),
        }
    }
}

fn d_web_safe_search() -> String { "moderate".to_string() }
fn d_web_max_results() -> usize { 10 }
fn d_web_request_timeout() -> u64 { 10 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    pub enabled: bool,
    pub max_results: usize,
    pub request_timeout_secs: u64,
    pub safe_search: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_results: d_web_max_results(),
            request_timeout_secs: d_web_request_timeout(),
            safe_search: d_web_safe_search(),
        }
    }
}

fn d_context7_api_url() -> String { "https://context7.com/api".to_string() }
fn d_context7_request_timeout() -> u64 { 30 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Context7Config {
    pub enabled: bool,
    pub api_url: String,
    pub request_timeout_secs: u64,
}

impl Default for Context7Config {
    fn default() -> Self {
        Self {
            enabled: false,
            api_url: d_context7_api_url(),
            request_timeout_secs: d_context7_request_timeout(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SqliteConfig {
    pub enabled: bool,
    /// Maximum rows returned by sqlite.query (hard cap: 10 000).
    pub max_result_rows: usize,
    /// Per-statement wall-clock timeout in seconds.
    pub statement_timeout_secs: u64,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self { enabled: false, max_result_rows: 1_000, statement_timeout_secs: 30 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DbConfig {
    pub enabled: bool,
    /// Maximum rows returned by db.query / db.sample (hard cap: 10 000).
    pub max_result_rows: usize,
    /// Refuse files larger than this (bytes). Default 100 MiB.
    pub max_file_bytes: usize,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self { enabled: false, max_result_rows: 1_000, max_file_bytes: 100 * 1024 * 1024 }
    }
}

fn d_pandoc_timeout() -> u64 { 30 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DocConfig {
    pub enabled: bool,
    /// Path to the pandoc binary. Empty means resolve from PATH at startup.
    pub pandoc_bin: String,
    /// Timeout in seconds for one pandoc subprocess call.
    pub pandoc_timeout_secs: u64,
}
impl Default for DocConfig {
    fn default() -> Self {
        Self { enabled: false, pandoc_bin: String::new(), pandoc_timeout_secs: d_pandoc_timeout() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub server: HttpConfig,
    pub auth: AuthConfig,
    pub infra: InfraConfig,
    pub safety: SafetyConfig,
    pub extract: ExtractConfig,
    pub api: ApiConfig,
    pub git: GitConfig,
    pub web: WebConfig,
    pub context7: Context7Config,
    pub sqlite: SqliteConfig,
    pub db: DbConfig,
    pub doc: DocConfig,
}

impl ServerConfig {
    /// Load and parse a YAML config, expanding `${VAR}` / `${VAR:-default}` first.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|e| {
            ToolError::invalid_argument(format!(
                "configuration file not found: {} ({e})",
                path.display()
            ))
        })?;
        Self::from_yaml(&raw)
    }

    pub fn from_yaml(raw: &str) -> Result<Self> {
        let expanded = expand_env(raw);
        let config: Self = serde_yaml::from_str(&expanded)
            .map_err(|e| ToolError::invalid_argument(format!("invalid config: {e}")))?;
        // Validation is part of loading: a misconfigured backend must stop the boot
        // rather than surface as a failed request once traffic arrives.
        config.validate()?;
        Ok(config)
    }

    /// Reject a configuration that cannot work, naming the offending key.
    pub fn validate(&self) -> Result<()> {
        validate_store("meta", &self.infra.meta.backend, &self.infra.meta.dsn)?;
        validate_store("admin", &self.infra.admin.backend, &self.infra.admin.dsn)?;
        validate_store("git", &self.infra.git.backend, &self.infra.git.dsn)?;
        validate_store("oauth", &self.infra.oauth.backend, &self.infra.oauth.dsn)?;
        Ok(())
    }

    // ── path helpers (mirror the C# ConfigLoader helpers) ────────────────────

    /// `state/volumes/{project_id}.db`
    pub fn volume_meta_path(&self, project_id: &str) -> PathBuf {
        Path::new(&self.infra.meta.dir).join(format!("{project_id}.db"))
    }

    /// `{bucket_prefix}{project_id}`
    pub fn volume_bucket(&self, project_id: &str) -> String {
        format!("{}{}", self.infra.blob.bucket_prefix, project_id)
    }

    pub fn admin_db_path(&self) -> PathBuf {
        PathBuf::from(&self.infra.admin.path)
    }

    /// The state root, derived from the metadata dir's parent (`state/`).
    pub fn state_root(&self) -> PathBuf {
        Path::new(&self.infra.meta.dir)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("state"))
    }

    /// `state/git/{project_id}.db`
    pub fn git_db_path(&self, project_id: &str) -> PathBuf {
        self.state_root().join("git").join(format!("{project_id}.db"))
    }

    /// `state/git-repos/{project_id}/`
    pub fn git_repo_dir(&self, project_id: &str) -> PathBuf {
        self.state_root().join("git-repos").join(project_id)
    }

    /// `state/oauth.db`
    pub fn oauth_db_path(&self) -> PathBuf {
        self.state_root().join("oauth.db")
    }

    /// Caseless platform-admin check.
    pub fn is_admin(&self, person: &str) -> bool {
        let p = crate::util::normalize_identity(person);
        self.auth.admins.iter().any(|a| crate::util::normalize_identity(a) == p)
    }
}

/// Check one `infra.<section>` store block.
///
/// The three failures below are all silent misconfigurations otherwise: a DSN that
/// is never read, a backend that cannot connect because its DSN is missing, and a
/// backend whose driver was never compiled in.
fn validate_store(section: &str, backend: &str, dsn: &Dsn) -> Result<()> {
    match backend {
        backend::SQLITE => {
            if !dsn.is_empty() {
                return Err(ToolError::invalid_argument(format!(
                    "infra.{section}.dsn is set but infra.{section}.backend is 'sqlite', \
                     which stores to a file and never reads a dsn: remove one of the two"
                )));
            }
            Ok(())
        }
        backend::POSTGRES => {
            if dsn.is_empty() {
                return Err(ToolError::invalid_argument(format!(
                    "infra.{section}.dsn is required when infra.{section}.backend is 'postgres'"
                )));
            }
            if !cfg!(feature = "postgres") {
                return Err(ToolError::invalid_argument(format!(
                    "infra.{section}.backend is 'postgres' but this binary was built without \
                     the 'postgres' cargo feature: rebuild with --features postgres"
                )));
            }
            Ok(())
        }
        backend::SQLSERVER => {
            if dsn.is_empty() {
                return Err(ToolError::invalid_argument(format!(
                    "infra.{section}.dsn is required when infra.{section}.backend is 'sqlserver'"
                )));
            }
            if !cfg!(feature = "sqlserver") {
                return Err(ToolError::invalid_argument(format!(
                    "infra.{section}.backend is 'sqlserver' but this binary was built without \
                     the 'sqlserver' cargo feature: rebuild with --features sqlserver"
                )));
            }
            Ok(())
        }
        other => Err(ToolError::invalid_argument(format!(
            "unknown infra.{section}.backend '{other}', expected one of {}",
            backend::ALL.join(", ")
        ))),
    }
}

/// Substitute `${VAR}` and `${VAR:-default}` from the environment.
/// An unset variable with no default expands to the empty string.
pub fn expand_env(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{'
            && let Some(end) = text[i + 2..].find('}') {
                let inner = &text[i + 2..i + 2 + end];
                let (name, default) = match inner.find(":-") {
                    Some(p) => (&inner[..p], Some(&inner[p + 2..])),
                    None => (inner, None),
                };
                let valid = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !name.starts_with(|c: char| c.is_ascii_digit());
                if valid {
                    let val = std::env::var(name).ok().filter(|v| !v.is_empty());
                    match (val, default) {
                        (Some(v), _) => out.push_str(&v),
                        (None, Some(d)) => out.push_str(d),
                        (None, None) => {}
                    }
                    i = i + 2 + end + 1;
                    continue;
                }
            }
        // push the raw byte sequence for this char
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_csharp() {
        let c = ServerConfig::default();
        assert_eq!(c.server.host, "0.0.0.0");
        assert_eq!(c.server.port, 5002);
        assert_eq!(c.server.mcp_path, "/mcp");
        assert_eq!(c.auth.jwt.header, "X-Forwarded-Authorization");
        assert_eq!(c.auth.jwt.algorithms, vec!["RS256"]);
        assert_eq!(c.auth.jwt.issuer.as_deref(), Some("web-a2a"));
        assert_eq!(c.auth.jwt.username_claim, "email");
        assert_eq!(c.infra.meta.dir, "state/volumes");
        assert_eq!(c.infra.blob.backend, "local");
        assert_eq!(c.infra.blob.dir, "state/blobs");
        assert_eq!(c.infra.blob.bucket_prefix, "mcpfs-");
        assert_eq!(c.infra.blob.region, "us-east-1");
        assert_eq!(c.infra.admin.path, "state/admin.db");
        assert_eq!(c.safety.write_quota_bytes, 52_428_800);
        assert_eq!(c.safety.trash_dir, ".mcp_trash");
        assert!(c.safety.read_guard);
        assert!(!c.safety.allow_hard_delete);
        assert_eq!(c.safety.max_read_lines, 2000);
        assert!(c.api.enabled);
        assert_eq!(c.extract.ocr.provider, "none");
        assert!(!c.git.enabled);
        assert_eq!(c.git.object_format, "sha1");
        assert_eq!(c.git.max_pack_size_mb, 512);
        assert_eq!(c.git.github_client_secret_env, "MCPFS_GITHUB_CLIENT_SECRET");
        assert_eq!(c.git.gitlab_instance_url, "https://gitlab.com");
    }

    #[test]
    fn expand_env_uses_variable_when_set() {
        unsafe { std::env::set_var("MCPFS_TEST_X", "secret123") };
        assert_eq!(expand_env("key: ${MCPFS_TEST_X}"), "key: secret123");
        unsafe { std::env::remove_var("MCPFS_TEST_X") };
    }

    #[test]
    fn expand_env_uses_default_when_unset() {
        unsafe { std::env::remove_var("MCPFS_TEST_UNSET") };
        assert_eq!(
            expand_env("url: ${MCPFS_TEST_UNSET:-http://localhost}"),
            "url: http://localhost"
        );
    }

    #[test]
    fn expand_env_variable_overrides_default() {
        unsafe { std::env::set_var("MCPFS_TEST_Y", "real") };
        assert_eq!(expand_env("v: ${MCPFS_TEST_Y:-fallback}"), "v: real");
        unsafe { std::env::remove_var("MCPFS_TEST_Y") };
    }

    #[test]
    fn expand_env_missing_without_default_is_empty() {
        unsafe { std::env::remove_var("MCPFS_TEST_MISSING") };
        assert_eq!(expand_env("v: ${MCPFS_TEST_MISSING}"), "v: ");
    }

    #[test]
    fn expand_env_leaves_non_variables_alone() {
        assert_eq!(expand_env("plain: value $notavar"), "plain: value $notavar");
        assert_eq!(expand_env("cost: 5$ and ${} weird"), "cost: 5$ and ${} weird");
    }

    #[test]
    fn parses_a_minio_config_with_env_secret() {
        unsafe { std::env::set_var("MCPFS_TEST_SECRET", "s3cr3t") };
        let yaml = r#"
server: { host: 127.0.0.1, port: 5003 }
infra:
  blob:
    backend: minio
    endpoint: "${MCPFS_TEST_ENDPOINT:-http://127.0.0.1:9000}"
    secret_key: "${MCPFS_TEST_SECRET}"
git:
  enabled: true
"#;
        let c = ServerConfig::from_yaml(yaml).unwrap();
        assert_eq!(c.server.port, 5003);
        assert_eq!(c.server.host, "127.0.0.1");
        // untouched keys keep their defaults
        assert_eq!(c.server.mcp_path, "/mcp");
        assert_eq!(c.infra.blob.backend, "minio");
        assert_eq!(c.infra.blob.endpoint, "http://127.0.0.1:9000");
        assert_eq!(c.infra.blob.secret_key, "s3cr3t");
        assert!(c.git.enabled);
        unsafe { std::env::remove_var("MCPFS_TEST_SECRET") };
    }

    #[test]
    fn path_helpers_match_csharp_layout() {
        let c = ServerConfig::default();
        assert_eq!(c.volume_meta_path("proj"), PathBuf::from("state/volumes/proj.db"));
        assert_eq!(c.volume_bucket("proj"), "mcpfs-proj");
        assert_eq!(c.admin_db_path(), PathBuf::from("state/admin.db"));
        assert_eq!(c.state_root(), PathBuf::from("state"));
        assert_eq!(c.git_db_path("proj"), PathBuf::from("state/git/proj.db"));
        assert_eq!(c.git_repo_dir("proj"), PathBuf::from("state/git-repos/proj"));
        assert_eq!(c.oauth_db_path(), PathBuf::from("state/oauth.db"));
    }

    // ── relational backends ──────────────────────────────────────────────────

    #[test]
    fn store_sections_default_to_sqlite_with_no_dsn() {
        let c = ServerConfig::default();
        for (section, backend, dsn) in [
            ("meta", &c.infra.meta.backend, &c.infra.meta.dsn),
            ("admin", &c.infra.admin.backend, &c.infra.admin.dsn),
            ("git", &c.infra.git.backend, &c.infra.git.dsn),
            ("oauth", &c.infra.oauth.backend, &c.infra.oauth.dsn),
        ] {
            assert_eq!(backend, backend::SQLITE, "infra.{section}.backend");
            assert!(dsn.is_empty(), "infra.{section}.dsn must start unset");
        }
        assert_eq!(c.infra.meta.schema, "public");
        assert_eq!(c.infra.meta.pool.max_connections, 10);
        assert_eq!(c.infra.meta.pool.acquire_timeout_secs, 30);
    }

    /// The whole point of the defaults: a config written before these keys existed
    /// still loads and still means exactly what it meant before.
    #[test]
    fn a_config_without_the_new_sections_still_loads() {
        let c = ServerConfig::from_yaml("infra:\n  meta:\n    dir: state/volumes\n").unwrap();
        assert_eq!(c.infra.git.backend, backend::SQLITE);
        assert_eq!(c.infra.oauth.backend, backend::SQLITE);
        assert_eq!(c.git_db_path("proj"), PathBuf::from("state/git/proj.db"));
        assert_eq!(c.oauth_db_path(), PathBuf::from("state/oauth.db"));
    }

    #[test]
    fn a_postgres_section_parses_and_keeps_its_pool_settings() {
        let yaml = r#"
infra:
  meta:
    backend: postgres
    dsn: "postgres://u:p@db:5432/mcpfs"
    schema: mcpfs
    pool: { max_connections: 25, acquire_timeout_secs: 5 }
"#;
        // Parsing must work regardless of which driver this binary compiled in,
        // so assert on the parsed shape and validate separately below.
        let parsed: ServerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.infra.meta.backend, backend::POSTGRES);
        assert_eq!(parsed.infra.meta.schema, "mcpfs");
        assert_eq!(parsed.infra.meta.pool.max_connections, 25);
        assert_eq!(parsed.infra.meta.pool.acquire_timeout_secs, 5);
        assert_eq!(parsed.infra.meta.dsn.expose(), "postgres://u:p@db:5432/mcpfs");
    }

    #[test]
    fn postgres_without_a_dsn_is_rejected_and_names_the_key() {
        let e = ServerConfig::from_yaml("infra:\n  meta:\n    backend: postgres\n")
            .expect_err("a postgres backend needs a dsn");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("infra.meta.dsn"), "{}", e.message);
    }

    /// A DSN that is silently ignored is how an operator ends up believing their
    /// data went to Postgres while it went to a local file.
    #[test]
    fn a_dsn_on_a_sqlite_backend_is_rejected() {
        let e = ServerConfig::from_yaml(
            "infra:\n  admin:\n    backend: sqlite\n    dsn: postgres://u:p@h/db\n",
        )
        .expect_err("a sqlite backend never reads a dsn");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("infra.admin.dsn"), "{}", e.message);
    }

    #[test]
    fn an_unknown_backend_lists_the_accepted_values() {
        let e = ServerConfig::from_yaml("infra:\n  git:\n    backend: mongodb\n")
            .expect_err("mongodb is not a relational backend");
        assert!(e.message.contains("infra.git.backend"), "{}", e.message);
        for name in backend::ALL {
            assert!(e.message.contains(name), "{name} should be listed: {}", e.message);
        }
    }

    /// A backend that needs a server cannot fall back to a file, so a missing DSN
    /// has to stop the boot rather than silently write somewhere else.
    #[test]
    fn sqlserver_without_a_dsn_is_rejected_and_names_the_key() {
        let e = ServerConfig::from_yaml("infra:\n  oauth:\n    backend: sqlserver\n")
            .expect_err("sqlserver cannot be reached without a dsn");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("infra.oauth.dsn"), "{}", e.message);
    }

    /// Only meaningful in a build that lacks the feature, which is the default.
    #[cfg(not(feature = "sqlserver"))]
    #[test]
    fn sqlserver_without_the_cargo_feature_names_the_feature() {
        let e = ServerConfig::from_yaml(
            "infra:\n  oauth:\n    backend: sqlserver\n    dsn: \"server=h;database=d\"\n",
        )
        .expect_err("the driver is not compiled in");
        assert!(e.message.contains("--features sqlserver"), "{}", e.message);
    }

    /// With the driver compiled in, a complete block is accepted at load time. The
    /// connection itself is only attempted when a store opens.
    #[cfg(feature = "sqlserver")]
    #[test]
    fn sqlserver_with_a_dsn_is_accepted_when_the_feature_is_on() {
        let c = ServerConfig::from_yaml(
            "infra:\n  oauth:\n    backend: sqlserver\n    dsn: \"server=h;database=d\"\n",
        )
        .expect("a complete sqlserver block is valid");
        assert_eq!(c.infra.oauth.backend, backend::SQLSERVER);
    }

    /// Only meaningful in a build that lacks the feature, which is the default.
    #[cfg(not(feature = "postgres"))]
    #[test]
    fn postgres_without_the_cargo_feature_names_the_feature() {
        let e = ServerConfig::from_yaml(
            "infra:\n  meta:\n    backend: postgres\n    dsn: postgres://u:p@h/db\n",
        )
        .expect_err("the driver is not compiled in");
        assert!(e.message.contains("--features postgres"), "{}", e.message);
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn postgres_with_the_cargo_feature_and_a_dsn_validates() {
        let c = ServerConfig::from_yaml(
            "infra:\n  meta:\n    backend: postgres\n    dsn: postgres://u:p@h/db\n",
        )
        .expect("a complete postgres section is valid");
        assert_eq!(c.infra.meta.backend, backend::POSTGRES);
    }

    // ── secret handling ─────────────────────────────────────────────────────────

    /// The worst failure mode in this feature is a password reaching a log line,
    /// so the guard is asserted on the whole rendered config, not just the field.
    #[test]
    fn a_dsn_password_never_appears_in_debug_or_display_output() {
        const PASSWORD: &str = "sup3rs3cr3t";
        let mut c = ServerConfig::default();
        c.infra.meta.dsn = Dsn::new(format!("postgres://user:{PASSWORD}@db:5432/mcpfs"));
        c.infra.admin.dsn = Dsn::new(format!("postgres://user:{PASSWORD}@db:5432/admin"));

        // `{:?}` on the whole config is what a tracing field or a panic prints.
        let debug = format!("{c:?}");
        assert!(!debug.contains(PASSWORD), "the password leaked into Debug: {debug}");
        assert!(debug.contains("Dsn(redacted)"), "a set dsn should say so: {debug}");

        let display = format!("{}", c.infra.meta.dsn);
        assert!(!display.contains(PASSWORD), "the password leaked into Display: {display}");

        // The driver still gets the real value, otherwise nothing could connect.
        assert!(c.infra.meta.dsn.expose().contains(PASSWORD));
    }

    #[test]
    fn an_unset_dsn_debugs_as_unset() {
        assert_eq!(format!("{:?}", Dsn::default()), "Dsn(unset)");
        assert_eq!(format!("{}", Dsn::default()), "unset");
        assert!(Dsn::new("   ").is_empty(), "whitespace only is unset");
    }

    /// A DSN belongs in an environment variable, never in a committed file, so the
    /// `${VAR}` path is the one that has to work.
    #[test]
    fn a_dsn_arrives_through_env_expansion() {
        unsafe { std::env::set_var("MCPFS_TEST_META_DSN", "postgres://u:pw@h/db") };
        let parsed: ServerConfig = serde_yaml::from_str(&expand_env(
            "infra:\n  meta:\n    backend: postgres\n    dsn: \"${MCPFS_TEST_META_DSN}\"\n",
        ))
        .unwrap();
        assert_eq!(parsed.infra.meta.dsn.expose(), "postgres://u:pw@h/db");
        unsafe { std::env::remove_var("MCPFS_TEST_META_DSN") };
    }

    #[test]
    fn is_admin_is_caseless() {
        let mut c = ServerConfig::default();
        c.auth.admins = vec!["Admin@Example.COM".into()];
        assert!(c.is_admin("admin@example.com"));
        assert!(c.is_admin("ADMIN@EXAMPLE.COM"));
        assert!(!c.is_admin("someone@else.com"));
    }
}
