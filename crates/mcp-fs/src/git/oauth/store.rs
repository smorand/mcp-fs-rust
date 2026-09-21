//! OAuth bearer token store.
//!
//! Port of the C# `Git/OAuth/OAuthTokenStore.cs`. In memory by default, keyed
//! `"{person}:{host}"` lowercased so identity casing never splits a session.
//! The key is `(person, host)`, not `(person, provider)`: one person may hold
//! distinct tokens for `github.com` and `github.ibm.com`, both provider
//! `github`, which a provider-only key could never tell apart (DEC-003,
//! DEC-004). `provider` is retained as a non-key attribute of the stored
//! session.
//!
//! When `MCPFS_TOKEN_KEY` is set the store is backed by
//! [`SqliteOAuthPersistence`]: sessions are loaded at startup and every mutation
//! is written through, encrypted, so authentication survives a restart.
//!
//! Tokens are never logged and never included in `Debug` output.

use crate::config::ServerConfig;
use crate::errors::Result;
use crate::git::oauth::cipher;
use crate::git::oauth::persistence::RelationalOAuthPersistence;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// The environment variable holding the base64 AES-256 key.
pub const TOKEN_KEY_ENV: &str = "MCPFS_TOKEN_KEY";

/// An active OAuth session for one person plus host.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthSession {
    /// The credential policy this host resolved to when the token was seeded.
    /// Not part of the session's identity: see the module docs.
    pub provider: String,
    pub access_token: String,
    pub scopes: Vec<String>,
    /// `None` means the token never expires (the Implementer Decision on this
    /// representation: a relaxed nullable column rather than a far-future
    /// sentinel).
    pub expires_at: Option<DateTime<Utc>>,
    /// Self hosted GitLab base URL, `None` for github.com.
    pub instance_url: Option<String>,
}

impl OAuthSession {
    pub fn is_valid_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_none_or(|e| e > now)
    }
}

/// Redacts the token: a session must never leak through a log line.
impl std::fmt::Debug for OAuthSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthSession")
            .field("provider", &self.provider)
            .field("access_token", &"<redacted>")
            .field("scopes", &self.scopes)
            .field("expires_at", &self.expires_at)
            .field("instance_url", &self.instance_url)
            .finish()
    }
}

/// Sessions plus the original casing of the ids, needed to write through to
/// persistence with the same person/host strings the caller used.
struct Entry {
    person: String,
    host: String,
    session: OAuthSession,
}

pub struct OAuthTokenStore {
    sessions: RwLock<HashMap<String, Entry>>,
    persistence: Option<Arc<RelationalOAuthPersistence>>,
}

fn key(person: &str, host: &str) -> String {
    format!("{}:{}", person.to_lowercase(), host.to_lowercase())
}

impl Default for OAuthTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuthTokenStore {
    /// Memory only store.
    pub fn new() -> Self {
        Self { sessions: RwLock::new(HashMap::new()), persistence: None }
    }

    /// Store backed by encrypted persistence, preloaded from it.
    pub async fn with_persistence(persistence: Arc<RelationalOAuthPersistence>) -> Result<Self> {
        let mut sessions = HashMap::new();
        for (person, host, session) in persistence.load_all().await? {
            sessions.insert(key(&person, &host), Entry { person, host, session });
        }
        Ok(Self { sessions: RwLock::new(sessions), persistence: Some(persistence) })
    }

    /// The composition root entry point: persistent when `MCPFS_TOKEN_KEY` is set
    /// and decodes to 32 bytes, memory only otherwise. A malformed key is an error
    /// rather than a silent downgrade, so a typo cannot quietly lose tokens.
    /// Takes the process wide registry rather than building one: on a server
    /// backend the persistence holds its pool open for the life of the store, so a
    /// private registry here would be a second pool against the configured budget.
    pub async fn from_env(
        config: &ServerConfig,
        registry: &crate::storage::RelationalRegistry,
    ) -> Result<Self> {
        match std::env::var(TOKEN_KEY_ENV) {
            Ok(raw) if !raw.trim().is_empty() => {
                let k = cipher::decode_key(&raw)?;
                let p = crate::storage::build_oauth_persistence(config, registry, k).await?;
                Self::with_persistence(p).await
            }
            _ => Ok(Self::new()),
        }
    }

    /// True when tokens are persisted to encrypted storage.
    pub fn is_persistent(&self) -> bool {
        self.persistence.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn store_token(
        &self,
        person: &str,
        host: &str,
        provider: &str,
        access_token: &str,
        scopes: Vec<String>,
        expires_at: Option<DateTime<Utc>>,
        instance_url: Option<String>,
    ) -> Result<()> {
        let session = OAuthSession {
            provider: provider.to_string(),
            access_token: access_token.to_string(),
            scopes,
            expires_at,
            instance_url,
        };
        {
            let mut guard = self.sessions.write().expect("token store lock poisoned");
            guard.insert(
                key(person, host),
                Entry {
                    person: person.to_string(),
                    host: host.to_string(),
                    session: session.clone(),
                },
            );
        }
        if let Some(p) = &self.persistence {
            p.upsert(person, host, &session).await?;
        }
        Ok(())
    }

    pub fn get_token(&self, person: &str, host: &str) -> Option<OAuthSession> {
        let guard = self.sessions.read().expect("token store lock poisoned");
        guard.get(&key(person, host)).map(|e| e.session.clone())
    }

    pub async fn revoke_token(&self, person: &str, host: &str) -> Result<()> {
        let removed = {
            let mut guard = self.sessions.write().expect("token store lock poisoned");
            guard.remove(&key(person, host))
        };
        if let Some(p) = &self.persistence {
            // Delete with the stored casing when known, so the row really goes.
            match &removed {
                Some(e) => p.delete(&e.person, &e.host).await?,
                None => p.delete(person, host).await?,
            }
        }
        Ok(())
    }

    /// A stored token that has not expired yet.
    pub fn has_valid_token(&self, person: &str, host: &str) -> bool {
        self.get_token(person, host).is_some_and(|s| s.is_valid_at(Utc::now()))
    }

    /// Every `(person, host)` currently held, original casing. Diagnostics.
    pub fn list_ids(&self) -> Vec<(String, String)> {
        let guard = self.sessions.read().expect("token store lock poisoned");
        let mut out: Vec<(String, String)> =
            guard.values().map(|e| (e.person.clone(), e.host.clone())).collect();
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn future() -> DateTime<Utc> {
        Utc::now() + chrono::Duration::hours(1)
    }

    fn past() -> DateTime<Utc> {
        Utc::now() - chrono::Duration::minutes(1)
    }

    fn store() -> OAuthTokenStore {
        OAuthTokenStore::new()
    }

    #[tokio::test]
    async fn store_then_get() {
        let s = store();
        s.store_token(
            "alice@test.com",
            "github.com",
            "github",
            "gho_1",
            vec!["repo".into()],
            Some(future()),
            None,
        )
        .await
        .unwrap();
        let got = s.get_token("alice@test.com", "github.com").unwrap();
        assert_eq!(got.access_token, "gho_1");
        assert_eq!(got.provider, "github");
        assert_eq!(got.scopes, vec!["repo"]);
        assert!(got.instance_url.is_none());
        assert!(!s.is_persistent());
    }

    #[tokio::test]
    async fn keying_is_caseless_on_both_parts() {
        let s = store();
        s.store_token(
            "Alice@Test.COM",
            "GitHub.com",
            "github",
            "tok",
            vec![],
            Some(future()),
            None,
        )
        .await
        .unwrap();
        assert!(s.get_token("alice@test.com", "github.com").is_some());
        assert!(s.get_token("ALICE@TEST.COM", "GITHUB.COM").is_some());
        assert!(s.get_token("alice@test.com", "gitlab.com").is_none());
        assert!(s.get_token("bob@test.com", "github.com").is_none());
        assert_eq!(key("A@B.C", "GitHub.com"), "a@b.c:github.com");
    }

    #[tokio::test]
    async fn store_overwrites_the_same_key_regardless_of_casing() {
        let s = store();
        s.store_token("a@t.c", "github.com", "github", "first", vec![], Some(future()), None)
            .await
            .unwrap();
        s.store_token("A@T.C", "GITHUB.COM", "github", "second", vec![], Some(future()), None)
            .await
            .unwrap();
        assert_eq!(s.get_token("a@t.c", "github.com").unwrap().access_token, "second");
        assert_eq!(s.list_ids().len(), 1);
    }

    /// Renamed from `providers_are_independent` (E2E-MOD-001): also proves
    /// E2E-NEW-032, the collision the old `(person, provider)` key made
    /// impossible: two hosts of the SAME provider hold independent tokens.
    #[tokio::test]
    async fn hosts_are_independent() {
        let s = store();
        s.store_token("a@t.c", "github.com", "github", "gh-public", vec![], Some(future()), None)
            .await
            .unwrap();
        s.store_token(
            "a@t.c",
            "github.ibm.com",
            "github",
            "gh-enterprise",
            vec!["api".into()],
            Some(future()),
            Some("https://github.ibm.com".into()),
        )
        .await
        .unwrap();
        assert_eq!(s.get_token("a@t.c", "github.com").unwrap().access_token, "gh-public");
        let ent = s.get_token("a@t.c", "github.ibm.com").unwrap();
        assert_eq!(ent.access_token, "gh-enterprise");
        assert_eq!(ent.provider, "github", "both hosts share the same provider");
        assert_eq!(ent.instance_url.as_deref(), Some("https://github.ibm.com"));
        assert_eq!(
            s.list_ids(),
            vec![
                ("a@t.c".to_string(), "github.com".to_string()),
                ("a@t.c".to_string(), "github.ibm.com".to_string()),
            ]
        );
    }

    /// E2E-NEW-033: a token seeded for `GitHub.IBM.com` is retrievable under any
    /// casing of that host.
    #[tokio::test]
    async fn e2e_new_033_host_matched_case_insensitively_on_seeding() {
        let s = store();
        s.store_token("a@t.c", "GitHub.IBM.com", "github", "tok", vec![], Some(future()), None)
            .await
            .unwrap();
        assert!(s.get_token("a@t.c", "github.ibm.com").is_some());
        assert!(s.get_token("a@t.c", "GITHUB.IBM.COM").is_some());
        assert_eq!(
            s.list_ids(),
            vec![("a@t.c".to_string(), "GitHub.IBM.com".to_string())],
            "the original casing is preserved for diagnostics"
        );
    }

    #[tokio::test]
    async fn has_valid_token_respects_expiry() {
        let s = store();
        s.store_token("a@t.c", "github.com", "github", "fresh", vec![], Some(future()), None)
            .await
            .unwrap();
        assert!(s.has_valid_token("a@t.c", "github.com"));
        assert!(s.has_valid_token("A@T.C", "GitHub.com"), "expiry check is caseless too");

        s.store_token("b@t.c", "github.com", "github", "stale", vec![], Some(past()), None)
            .await
            .unwrap();
        assert!(!s.has_valid_token("b@t.c", "github.com"));
        // an expired session is still retrievable, only "valid" is false
        assert_eq!(s.get_token("b@t.c", "github.com").unwrap().access_token, "stale");
        assert!(!s.has_valid_token("nobody@t.c", "github.com"));

        // a `None` expiry (the Implementer Decision's representation) never expires
        s.store_token("c@t.c", "github.com", "github", "forever", vec![], None, None)
            .await
            .unwrap();
        assert!(s.has_valid_token("c@t.c", "github.com"));
    }

    #[tokio::test]
    async fn revoke_removes_the_session() {
        let s = store();
        s.store_token("a@t.c", "github.com", "github", "tok", vec![], Some(future()), None)
            .await
            .unwrap();
        s.revoke_token("A@T.C", "GITHUB.COM").await.unwrap();
        assert!(s.get_token("a@t.c", "github.com").is_none());
        assert!(!s.has_valid_token("a@t.c", "github.com"));
        // revoking twice is a no-op
        s.revoke_token("a@t.c", "github.com").await.unwrap();
    }

    /// E2E-NEW-082 / E2E-NEW-162: two people each hold their own token for the
    /// SAME host, and a lookup for one never returns the other's. This is the
    /// store level guarantee that credential resolution for a push or a clone
    /// (later stories) rests on: the pusher's own token, never the cloner's.
    #[tokio::test]
    async fn two_people_hold_independent_tokens_for_one_host() {
        let s = store();
        s.store_token(
            "alice@test.com",
            "github.ibm.com",
            "github",
            "alice-tok",
            vec![],
            Some(future()),
            None,
        )
        .await
        .unwrap();
        s.store_token(
            "bob@test.com",
            "github.ibm.com",
            "github",
            "bob-tok",
            vec![],
            Some(future()),
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            s.get_token("alice@test.com", "github.ibm.com").unwrap().access_token,
            "alice-tok"
        );
        assert_eq!(s.get_token("bob@test.com", "github.ibm.com").unwrap().access_token, "bob-tok");
        assert_ne!(
            s.get_token("alice@test.com", "github.ibm.com").unwrap().access_token,
            s.get_token("bob@test.com", "github.ibm.com").unwrap().access_token,
            "the credential supplied for one person must never be the other's"
        );
    }

    /// E2E-NEW-163: revocation by one person never touches another's token for
    /// the same host.
    #[tokio::test]
    async fn revocation_by_one_person_does_not_affect_another() {
        let s = store();
        s.store_token(
            "alice@test.com",
            "github.ibm.com",
            "github",
            "alice-tok",
            vec![],
            Some(future()),
            None,
        )
        .await
        .unwrap();
        s.store_token(
            "bob@test.com",
            "github.ibm.com",
            "github",
            "bob-tok",
            vec![],
            Some(future()),
            None,
        )
        .await
        .unwrap();

        s.revoke_token("alice@test.com", "github.ibm.com").await.unwrap();
        assert!(s.get_token("alice@test.com", "github.ibm.com").is_none());
        assert_eq!(s.get_token("bob@test.com", "github.ibm.com").unwrap().access_token, "bob-tok");
        assert!(s.has_valid_token("bob@test.com", "github.ibm.com"), "bob still authenticates");
    }

    #[test]
    fn debug_output_redacts_the_token() {
        let session = OAuthSession {
            provider: "github".into(),
            access_token: "gho_verysecret".into(),
            scopes: vec!["repo".into()],
            expires_at: Some(future()),
            instance_url: None,
        };
        let dbg = format!("{session:?}");
        assert!(!dbg.contains("gho_verysecret"), "tokens must never be logged");
        assert!(dbg.contains("<redacted>"));
    }

    /// SQLite backed persistence at a real path, so a restart can be simulated.
    async fn open_persistence(
        path: &std::path::Path,
        k: [u8; cipher::KEY_SIZE],
    ) -> Arc<RelationalOAuthPersistence> {
        let db = Arc::new(crate::storage::rel::SqliteRelationalDb::open(path).unwrap());
        Arc::new(RelationalOAuthPersistence::open(db, k).await.unwrap())
    }

    #[tokio::test]
    async fn persistent_store_loads_and_writes_through() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/oauth.db");
        let k = [42u8; cipher::KEY_SIZE];

        {
            let p = open_persistence(&path, k).await;
            let s = OAuthTokenStore::with_persistence(p).await.unwrap();
            assert!(s.is_persistent());
            s.store_token(
                "Alice@Test.com",
                "GitHub.com",
                "github",
                "gho_persisted",
                vec!["repo".into()],
                Some(future()),
                None,
            )
            .await
            .unwrap();
        }

        // a restart must find the session again, keyed caselessly
        let p2 = open_persistence(&path, k).await;
        let s2 = OAuthTokenStore::with_persistence(p2.clone()).await.unwrap();
        let got = s2.get_token("alice@test.com", "GITHUB.COM").unwrap();
        assert_eq!(got.access_token, "gho_persisted");
        assert!(s2.has_valid_token("alice@test.com", "github.com"));
        assert_eq!(
            s2.list_ids(),
            vec![("Alice@Test.com".to_string(), "GitHub.com".to_string())],
            "the original casing is preserved for write through"
        );

        // revoke must clear the row too
        s2.revoke_token("alice@test.com", "github.com").await.unwrap();
        assert_eq!(p2.count().await.unwrap(), 0);
        let s3 = OAuthTokenStore::with_persistence(p2).await.unwrap();
        assert!(s3.get_token("alice@test.com", "github.com").is_none());
    }

    #[tokio::test]
    async fn from_env_is_memory_only_without_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = ServerConfig::default();
        c.infra.meta.dir = dir.path().join("state/volumes").display().to_string();
        // The env var is process wide, so assert on the absent case only when unset.
        if std::env::var(TOKEN_KEY_ENV).is_err() {
            let reg = crate::storage::RelationalRegistry::new();
            let s = OAuthTokenStore::from_env(&c, &reg).await.unwrap();
            assert!(!s.is_persistent());
            assert!(!dir.path().join("state/oauth.db").exists());
        }
    }
}
