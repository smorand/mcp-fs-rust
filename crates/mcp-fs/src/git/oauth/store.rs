//! OAuth bearer token store.
//!
//! Port of the C# `Git/OAuth/OAuthTokenStore.cs`. In memory by default, keyed
//! `"{person}:{provider}"` lowercased so identity casing never splits a session.
//! When `MCPFS_TOKEN_KEY` is set the store is backed by
//! [`SqliteOAuthPersistence`]: sessions are loaded at startup and every mutation
//! is written through, encrypted, so authentication survives a restart.
//!
//! Tokens are never logged and never included in `Debug` output.

use crate::config::ServerConfig;
use crate::errors::Result;
use crate::git::oauth::cipher;
use crate::git::oauth::persistence::SqliteOAuthPersistence;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// The environment variable holding the base64 AES-256 key.
pub const TOKEN_KEY_ENV: &str = "MCPFS_TOKEN_KEY";

/// An active OAuth session for one person plus provider.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthSession {
    pub provider: String,
    pub access_token: String,
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
    /// Self hosted GitLab base URL, `None` for github.com.
    pub instance_url: Option<String>,
}

impl OAuthSession {
    pub fn is_valid_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at > now
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
/// persistence with the same person/provider strings the caller used.
struct Entry {
    person: String,
    provider: String,
    session: OAuthSession,
}

pub struct OAuthTokenStore {
    sessions: RwLock<HashMap<String, Entry>>,
    persistence: Option<Arc<SqliteOAuthPersistence>>,
}

fn key(person: &str, provider: &str) -> String {
    format!("{}:{}", person.to_lowercase(), provider.to_lowercase())
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
    pub fn with_persistence(persistence: Arc<SqliteOAuthPersistence>) -> Result<Self> {
        let mut sessions = HashMap::new();
        for (person, provider, session) in persistence.load_all()? {
            sessions.insert(
                key(&person, &provider),
                Entry { person, provider, session },
            );
        }
        Ok(Self { sessions: RwLock::new(sessions), persistence: Some(persistence) })
    }

    /// The composition root entry point: persistent when `MCPFS_TOKEN_KEY` is set
    /// and decodes to 32 bytes, memory only otherwise. A malformed key is an error
    /// rather than a silent downgrade, so a typo cannot quietly lose tokens.
    pub fn from_env(config: &ServerConfig) -> Result<Self> {
        match std::env::var(TOKEN_KEY_ENV) {
            Ok(raw) if !raw.trim().is_empty() => {
                let k = cipher::decode_key(&raw)?;
                let p = SqliteOAuthPersistence::open(config.oauth_db_path(), k)?;
                Self::with_persistence(Arc::new(p))
            }
            _ => Ok(Self::new()),
        }
    }

    /// True when tokens are persisted to encrypted storage.
    pub fn is_persistent(&self) -> bool {
        self.persistence.is_some()
    }

    pub fn store_token(
        &self,
        person: &str,
        provider: &str,
        access_token: &str,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
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
                key(person, provider),
                Entry {
                    person: person.to_string(),
                    provider: provider.to_string(),
                    session: session.clone(),
                },
            );
        }
        if let Some(p) = &self.persistence {
            p.upsert(person, provider, &session)?;
        }
        Ok(())
    }

    pub fn get_token(&self, person: &str, provider: &str) -> Option<OAuthSession> {
        let guard = self.sessions.read().expect("token store lock poisoned");
        guard.get(&key(person, provider)).map(|e| e.session.clone())
    }

    pub fn revoke_token(&self, person: &str, provider: &str) -> Result<()> {
        let removed = {
            let mut guard = self.sessions.write().expect("token store lock poisoned");
            guard.remove(&key(person, provider))
        };
        if let Some(p) = &self.persistence {
            // Delete with the stored casing when known, so the row really goes.
            match &removed {
                Some(e) => p.delete(&e.person, &e.provider)?,
                None => p.delete(person, provider)?,
            }
        }
        Ok(())
    }

    /// A stored token that has not expired yet.
    pub fn has_valid_token(&self, person: &str, provider: &str) -> bool {
        self.get_token(person, provider)
            .is_some_and(|s| s.is_valid_at(Utc::now()))
    }

    /// Every `(person, provider)` currently held, original casing. Diagnostics.
    pub fn list_ids(&self) -> Vec<(String, String)> {
        let guard = self.sessions.read().expect("token store lock poisoned");
        let mut out: Vec<(String, String)> = guard
            .values()
            .map(|e| (e.person.clone(), e.provider.clone()))
            .collect();
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

    #[test]
    fn store_then_get() {
        let s = store();
        s.store_token(
            "alice@test.com",
            "github",
            "gho_1",
            vec!["repo".into()],
            future(),
            None,
        )
        .unwrap();
        let got = s.get_token("alice@test.com", "github").unwrap();
        assert_eq!(got.access_token, "gho_1");
        assert_eq!(got.provider, "github");
        assert_eq!(got.scopes, vec!["repo"]);
        assert!(got.instance_url.is_none());
        assert!(!s.is_persistent());
    }

    #[test]
    fn keying_is_caseless_on_both_parts() {
        let s = store();
        s.store_token("Alice@Test.COM", "GitHub", "tok", vec![], future(), None).unwrap();
        assert!(s.get_token("alice@test.com", "github").is_some());
        assert!(s.get_token("ALICE@TEST.COM", "GITHUB").is_some());
        assert!(s.get_token("alice@test.com", "gitlab").is_none());
        assert!(s.get_token("bob@test.com", "github").is_none());
        assert_eq!(key("A@B.C", "GitHub"), "a@b.c:github");
    }

    #[test]
    fn store_overwrites_the_same_key_regardless_of_casing() {
        let s = store();
        s.store_token("a@t.c", "github", "first", vec![], future(), None).unwrap();
        s.store_token("A@T.C", "GITHUB", "second", vec![], future(), None).unwrap();
        assert_eq!(s.get_token("a@t.c", "github").unwrap().access_token, "second");
        assert_eq!(s.list_ids().len(), 1);
    }

    #[test]
    fn providers_are_independent() {
        let s = store();
        s.store_token("a@t.c", "github", "gh", vec![], future(), None).unwrap();
        s.store_token(
            "a@t.c",
            "gitlab",
            "gl",
            vec!["api".into()],
            future(),
            Some("https://gitlab.example.test".into()),
        )
        .unwrap();
        assert_eq!(s.get_token("a@t.c", "github").unwrap().access_token, "gh");
        let gl = s.get_token("a@t.c", "gitlab").unwrap();
        assert_eq!(gl.instance_url.as_deref(), Some("https://gitlab.example.test"));
        assert_eq!(s.list_ids(), vec![
            ("a@t.c".to_string(), "github".to_string()),
            ("a@t.c".to_string(), "gitlab".to_string()),
        ]);
    }

    #[test]
    fn has_valid_token_respects_expiry() {
        let s = store();
        s.store_token("a@t.c", "github", "fresh", vec![], future(), None).unwrap();
        assert!(s.has_valid_token("a@t.c", "github"));
        assert!(s.has_valid_token("A@T.C", "GitHub"), "expiry check is caseless too");

        s.store_token("b@t.c", "github", "stale", vec![], past(), None).unwrap();
        assert!(!s.has_valid_token("b@t.c", "github"));
        // an expired session is still retrievable, only "valid" is false
        assert_eq!(s.get_token("b@t.c", "github").unwrap().access_token, "stale");
        assert!(!s.has_valid_token("nobody@t.c", "github"));
    }

    #[test]
    fn revoke_removes_the_session() {
        let s = store();
        s.store_token("a@t.c", "github", "tok", vec![], future(), None).unwrap();
        s.revoke_token("A@T.C", "GITHUB").unwrap();
        assert!(s.get_token("a@t.c", "github").is_none());
        assert!(!s.has_valid_token("a@t.c", "github"));
        // revoking twice is a no-op
        s.revoke_token("a@t.c", "github").unwrap();
    }

    #[test]
    fn debug_output_redacts_the_token() {
        let session = OAuthSession {
            provider: "github".into(),
            access_token: "gho_verysecret".into(),
            scopes: vec!["repo".into()],
            expires_at: future(),
            instance_url: None,
        };
        let dbg = format!("{session:?}");
        assert!(!dbg.contains("gho_verysecret"), "tokens must never be logged");
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn persistent_store_loads_and_writes_through() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/oauth.db");
        let k = [42u8; cipher::KEY_SIZE];

        {
            let p = Arc::new(SqliteOAuthPersistence::open(&path, k).unwrap());
            let s = OAuthTokenStore::with_persistence(p).unwrap();
            assert!(s.is_persistent());
            s.store_token(
                "Alice@Test.com",
                "github",
                "gho_persisted",
                vec!["repo".into()],
                future(),
                None,
            )
            .unwrap();
        }

        // a restart must find the session again, keyed caselessly
        let p2 = Arc::new(SqliteOAuthPersistence::open(&path, k).unwrap());
        let s2 = OAuthTokenStore::with_persistence(p2.clone()).unwrap();
        let got = s2.get_token("alice@test.com", "GITHUB").unwrap();
        assert_eq!(got.access_token, "gho_persisted");
        assert!(s2.has_valid_token("alice@test.com", "github"));
        assert_eq!(
            s2.list_ids(),
            vec![("Alice@Test.com".to_string(), "github".to_string())],
            "the original casing is preserved for write through"
        );

        // revoke must clear the row too
        s2.revoke_token("alice@test.com", "github").unwrap();
        assert_eq!(p2.count().unwrap(), 0);
        let s3 = OAuthTokenStore::with_persistence(p2).unwrap();
        assert!(s3.get_token("alice@test.com", "github").is_none());
    }

    #[test]
    fn from_env_is_memory_only_without_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = ServerConfig::default();
        c.infra.meta.dir = dir.path().join("state/volumes").display().to_string();
        // The env var is process wide, so assert on the absent case only when unset.
        if std::env::var(TOKEN_KEY_ENV).is_err() {
            let s = OAuthTokenStore::from_env(&c).unwrap();
            assert!(!s.is_persistent());
            assert!(!dir.path().join("state/oauth.db").exists());
        }
    }
}
