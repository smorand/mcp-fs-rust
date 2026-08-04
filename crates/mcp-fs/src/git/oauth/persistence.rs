//! Encrypted at rest SQLite persistence for OAuth sessions.
//!
//! Port of the C# `Git/OAuth/SqliteOAuthPersistence.cs`. One row per
//! `(person, provider)` at `state/oauth.db`:
//!
//! ```sql
//! oauth_tokens(person, provider, token_enc BLOB, scopes, expires_at,
//!              instance_url, PRIMARY KEY (person, provider))
//! ```
//!
//! Only the bearer token is encrypted (AES-256-GCM, see [`super::cipher`]); the
//! metadata is stored in clear because none of it is a secret and it has to be
//! queryable. Instantiated only when `MCPFS_TOKEN_KEY` is set; without it the
//! token store is memory only and tokens are lost on restart.
//!
//! The methods are synchronous, like the C#: they run under the serialized
//! `SqliteDb` mutex, the rows are tiny, and the callers are rare (a device flow
//! completion, a revoke), so there is nothing to gain from async plumbing.

use crate::errors::Result;
use crate::git::oauth::cipher;
use crate::git::oauth::store::OAuthSession;
use crate::storage::sqlite::SqliteDb;
use chrono::{DateTime, Utc};
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS oauth_tokens (
    person       TEXT NOT NULL,
    provider     TEXT NOT NULL,
    token_enc    BLOB NOT NULL,
    scopes       TEXT NOT NULL,
    expires_at   TEXT NOT NULL,
    instance_url TEXT,
    PRIMARY KEY (person, provider)
);
";

pub struct SqliteOAuthPersistence {
    db: SqliteDb,
    key: [u8; cipher::KEY_SIZE],
}

impl SqliteOAuthPersistence {
    pub fn open(path: impl AsRef<Path>, key: [u8; cipher::KEY_SIZE]) -> Result<Self> {
        let db = SqliteDb::open(path)?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db, key })
    }

    /// In memory persistence, for tests.
    pub fn open_in_memory(key: [u8; cipher::KEY_SIZE]) -> Result<Self> {
        let db = SqliteDb::open_in_memory()?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db, key })
    }

    /// Every stored session. Rows that fail to decrypt (rotated key, corruption)
    /// are skipped: a bad row must never stop the server from starting.
    pub fn load_all(&self) -> Result<Vec<(String, String, OAuthSession)>> {
        self.db.run_sync(|tx| {
            let mut st = tx.prepare(
                "SELECT person, provider, token_enc, scopes, expires_at, instance_url \
                 FROM oauth_tokens ORDER BY person, provider",
            )?;
            let mut rows = st.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let person: String = r.get(0)?;
                let provider: String = r.get(1)?;
                let blob: Vec<u8> = r.get(2)?;
                let Ok(token) = cipher::decrypt(&self.key, &blob) else {
                    continue;
                };
                let scopes_raw: String = r.get(3)?;
                let expires_raw: String = r.get(4)?;
                let Ok(expires_at) = DateTime::parse_from_rfc3339(&expires_raw) else {
                    continue; // an unparseable timestamp is as unusable as a bad key
                };
                out.push((
                    person,
                    provider.clone(),
                    OAuthSession {
                        provider,
                        access_token: token,
                        scopes: split_scopes(&scopes_raw),
                        expires_at: expires_at.with_timezone(&Utc),
                        instance_url: r.get::<_, Option<String>>(5)?,
                    },
                ));
            }
            Ok(out)
        })
    }

    pub fn upsert(&self, person: &str, provider: &str, session: &OAuthSession) -> Result<()> {
        let enc = cipher::encrypt(&self.key, &session.access_token)?;
        let scopes = session.scopes.join(",");
        let expires = session.expires_at.to_rfc3339();
        let (person, provider, instance) =
            (person.to_string(), provider.to_string(), session.instance_url.clone());
        self.db.run_sync(move |tx| {
            tx.execute(
                "INSERT INTO oauth_tokens \
                     (person, provider, token_enc, scopes, expires_at, instance_url) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(person, provider) DO UPDATE SET \
                     token_enc    = excluded.token_enc, \
                     scopes       = excluded.scopes, \
                     expires_at   = excluded.expires_at, \
                     instance_url = excluded.instance_url",
                rusqlite::params![person, provider, enc, scopes, expires, instance],
            )?;
            Ok(())
        })
    }

    pub fn delete(&self, person: &str, provider: &str) -> Result<()> {
        let (person, provider) = (person.to_string(), provider.to_string());
        self.db.run_sync(move |tx| {
            tx.execute(
                "DELETE FROM oauth_tokens WHERE person = ?1 AND provider = ?2",
                rusqlite::params![person, provider],
            )?;
            Ok(())
        })
    }

    /// Row count, for diagnostics and tests.
    pub fn count(&self) -> Result<i64> {
        self.db
            .run_sync(|tx| Ok(tx.query_row("SELECT COUNT(*) FROM oauth_tokens", [], |r| r.get(0))?))
    }
}

/// Comma separated, empty entries dropped (C# `StringSplitOptions.RemoveEmptyEntries`).
fn split_scopes(raw: &str) -> Vec<String> {
    raw.split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> [u8; cipher::KEY_SIZE] {
        [b; cipher::KEY_SIZE]
    }

    fn session(token: &str) -> OAuthSession {
        OAuthSession {
            provider: "github".into(),
            access_token: token.into(),
            scopes: vec!["repo".into(), "read:user".into()],
            expires_at: DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            instance_url: None,
        }
    }

    #[test]
    fn upsert_then_load() {
        let p = SqliteOAuthPersistence::open_in_memory(key(1)).unwrap();
        p.upsert("alice@test.com", "github", &session("gho_abc")).unwrap();

        let all = p.load_all().unwrap();
        assert_eq!(all.len(), 1);
        let (person, provider, s) = &all[0];
        assert_eq!(person, "alice@test.com");
        assert_eq!(provider, "github");
        assert_eq!(s.access_token, "gho_abc");
        assert_eq!(s.scopes, vec!["repo", "read:user"]);
        assert_eq!(s.expires_at.to_rfc3339(), "2030-01-01T00:00:00+00:00");
        assert_eq!(s.instance_url, None);
    }

    #[test]
    fn upsert_replaces_the_row_for_the_same_key() {
        let p = SqliteOAuthPersistence::open_in_memory(key(2)).unwrap();
        p.upsert("bob@test.com", "gitlab", &session("first")).unwrap();
        let mut s = session("second");
        s.provider = "gitlab".into();
        s.scopes = vec!["api".into()];
        s.instance_url = Some("https://gitlab.example.test".into());
        p.upsert("bob@test.com", "gitlab", &s).unwrap();

        assert_eq!(p.count().unwrap(), 1, "primary key is (person, provider)");
        let all = p.load_all().unwrap();
        assert_eq!(all[0].2.access_token, "second");
        assert_eq!(all[0].2.scopes, vec!["api"]);
        assert_eq!(
            all[0].2.instance_url.as_deref(),
            Some("https://gitlab.example.test")
        );
    }

    #[test]
    fn one_row_per_person_provider_pair() {
        let p = SqliteOAuthPersistence::open_in_memory(key(3)).unwrap();
        p.upsert("a@t.c", "github", &session("t1")).unwrap();
        p.upsert("a@t.c", "gitlab", &session("t2")).unwrap();
        p.upsert("b@t.c", "github", &session("t3")).unwrap();
        assert_eq!(p.count().unwrap(), 3);
        let all = p.load_all().unwrap();
        assert_eq!(all.len(), 3);
        // ordered by person then provider
        assert_eq!(all[0].1, "github");
        assert_eq!(all[1].1, "gitlab");
        assert_eq!(all[2].0, "b@t.c");
    }

    #[test]
    fn delete_removes_one_row_and_is_idempotent() {
        let p = SqliteOAuthPersistence::open_in_memory(key(4)).unwrap();
        p.upsert("a@t.c", "github", &session("t1")).unwrap();
        p.upsert("a@t.c", "gitlab", &session("t2")).unwrap();
        p.delete("a@t.c", "github").unwrap();
        assert_eq!(p.count().unwrap(), 1);
        assert_eq!(p.load_all().unwrap()[0].1, "gitlab");
        p.delete("a@t.c", "github").unwrap();
        p.delete("nobody@t.c", "github").unwrap();
    }

    #[test]
    fn token_is_not_stored_in_clear() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/oauth.db");
        let p = SqliteOAuthPersistence::open(&path, key(5)).unwrap();
        p.upsert("a@t.c", "github", &session("gho_supersecret")).unwrap();

        let raw: Vec<u8> = p
            .db
            .run_sync(|tx| {
                Ok(tx.query_row("SELECT token_enc FROM oauth_tokens", [], |r| r.get(0))?)
            })
            .unwrap();
        assert!(
            !raw.windows(15).any(|w| w == b"gho_supersecret"),
            "the token must be ciphertext on disk"
        );
        assert_eq!(raw.len(), cipher::NONCE_SIZE + cipher::TAG_SIZE + 15);
    }

    #[test]
    fn rows_encrypted_with_another_key_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.db");
        {
            let old = SqliteOAuthPersistence::open(&path, key(6)).unwrap();
            old.upsert("stale@t.c", "github", &session("old_token")).unwrap();
        }
        // key rotation: the old row can no longer be decrypted
        let new = SqliteOAuthPersistence::open(&path, key(7)).unwrap();
        assert_eq!(new.count().unwrap(), 1, "the row is still there");
        assert!(
            new.load_all().unwrap().is_empty(),
            "but it is skipped instead of crashing startup"
        );

        // a new token under the new key loads fine alongside the unreadable one
        new.upsert("fresh@t.c", "github", &session("new_token")).unwrap();
        let all = new.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "fresh@t.c");
    }

    #[test]
    fn corrupt_blobs_and_bad_timestamps_are_skipped() {
        let p = SqliteOAuthPersistence::open_in_memory(key(8)).unwrap();
        p.upsert("good@t.c", "github", &session("ok")).unwrap();
        p.db
            .run_sync(|tx| {
                tx.execute(
                    "INSERT INTO oauth_tokens VALUES ('corrupt@t.c','github',?1,'repo','2030-01-01T00:00:00Z',NULL)",
                    rusqlite::params![vec![0u8; 40]],
                )?;
                tx.execute(
                    "INSERT INTO oauth_tokens VALUES ('badtime@t.c','github',?1,'repo','not-a-date',NULL)",
                    rusqlite::params![super::cipher::encrypt(&[8u8; 32], "tok").unwrap()],
                )?;
                Ok(())
            })
            .unwrap();

        assert_eq!(p.count().unwrap(), 3);
        let all = p.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "good@t.c");
    }

    #[test]
    fn scopes_round_trip_including_empty() {
        let p = SqliteOAuthPersistence::open_in_memory(key(9)).unwrap();
        let mut s = session("t");
        s.scopes = Vec::new();
        p.upsert("a@t.c", "github", &s).unwrap();
        assert!(p.load_all().unwrap()[0].2.scopes.is_empty());

        assert_eq!(split_scopes(""), Vec::<String>::new());
        assert_eq!(split_scopes("a,,b"), vec!["a", "b"], "empty entries dropped");
    }

    #[test]
    fn survives_reopen_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/oauth.db");
        {
            let p = SqliteOAuthPersistence::open(&path, key(10)).unwrap();
            p.upsert("a@t.c", "github", &session("persisted")).unwrap();
        }
        assert!(path.exists());
        let p2 = SqliteOAuthPersistence::open(&path, key(10)).unwrap();
        assert_eq!(p2.load_all().unwrap()[0].2.access_token, "persisted");
    }
}
