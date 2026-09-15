//! Encrypted at rest persistence for OAuth sessions, on any [`RelationalDb`].
//!
//! One row per `(person, provider)`. Only the bearer token is encrypted
//! (AES-256-GCM, see [`super::cipher`]); the metadata is stored in clear because
//! none of it is a secret and it has to be queryable.
//!
//! Instantiated only when `MCPFS_TOKEN_KEY` is set; without it the token store is
//! memory only and tokens are lost on restart.
//!
//! The methods are async because the PostgreSQL and SQL Server drivers are async
//! only. They used to be synchronous, which is why the token store above them had
//! to become async too.

use crate::errors::Result;
use crate::git::oauth::cipher;
use crate::git::oauth::store::OAuthSession;
use crate::storage::rel::dialect::{Assign, ColumnType, Upsert};
use crate::storage::rel::schema::{Column, SchemaSet, Table};
use crate::storage::rel::{Query, RelationalDb};
use chrono::{DateTime, Utc};
use std::sync::Arc;

/// An email comfortably fits, and both columns are part of the primary key.
const PERSON_LEN: u32 = 320;
const PROVIDER_LEN: u32 = 64;

/// The table this store owns.
pub fn schema() -> SchemaSet {
    SchemaSet::new(
        vec![Table::new(
            "oauth_tokens",
            vec![
                Column::required("person", ColumnType::TextKey(PERSON_LEN)),
                Column::required("provider", ColumnType::TextKey(PROVIDER_LEN)),
                // Ciphertext, so it is bytes on every engine: BLOB, BYTEA or
                // VARBINARY(MAX) depending on the dialect.
                Column::required("token_enc", ColumnType::Blob),
                Column::required("scopes", ColumnType::Text),
                Column::required("expires_at", ColumnType::Text),
                Column::new("instance_url", ColumnType::Text),
            ],
            vec!["person", "provider"],
        )],
        Vec::new(),
    )
}

pub struct RelationalOAuthPersistence {
    db: Arc<dyn RelationalDb>,
    key: [u8; cipher::KEY_SIZE],
}

impl RelationalOAuthPersistence {
    pub async fn open(db: Arc<dyn RelationalDb>, key: [u8; cipher::KEY_SIZE]) -> Result<Self> {
        let me = Self { db, key };
        me.db.migrate(&schema()).await?;
        Ok(me)
    }

    /// In memory persistence, for tests.
    pub async fn open_in_memory(key: [u8; cipher::KEY_SIZE]) -> Result<Self> {
        let db = Arc::new(crate::storage::rel::SqliteRelationalDb::open_in_memory()?);
        Self::open(db, key).await
    }

    /// Every stored session. Rows that fail to decrypt (rotated key, corruption)
    /// are skipped: a bad row must never stop the server from starting.
    pub async fn load_all(&self) -> Result<Vec<(String, String, OAuthSession)>> {
        let rows = self
            .db
            .query(&Query::new(
                "SELECT person, provider, token_enc, scopes, expires_at, instance_url \
                 FROM oauth_tokens ORDER BY person, provider",
            ))
            .await?;

        let mut out = Vec::new();
        for r in &rows {
            let person = r.text(0)?;
            let provider = r.text(1)?;
            let Ok(token) = cipher::decrypt(&self.key, &r.blob(2)?) else {
                continue;
            };
            let scopes_raw = r.text(3)?;
            let Ok(expires_at) = DateTime::parse_from_rfc3339(&r.text(4)?) else {
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
                    instance_url: r.opt_text(5)?,
                },
            ));
        }
        Ok(out)
    }

    pub async fn upsert(
        &self,
        person: &str,
        provider: &str,
        session: &OAuthSession,
    ) -> Result<()> {
        let enc = cipher::encrypt(&self.key, &session.access_token)?;
        let sql = self.db.dialect().render_upsert(&Upsert::update(
            "oauth_tokens",
            vec!["person", "provider", "token_enc", "scopes", "expires_at", "instance_url"],
            vec!["person", "provider"],
            vec![
                Assign::inserted("token_enc"),
                Assign::inserted("scopes"),
                Assign::inserted("expires_at"),
                Assign::inserted("instance_url"),
            ],
        ));
        self.db
            .execute(
                &Query::new(sql)
                    .bind(person)
                    .bind(provider)
                    .bind(enc)
                    .bind(session.scopes.join(","))
                    .bind(session.expires_at.to_rfc3339())
                    .bind(session.instance_url.clone()),
            )
            .await?;
        Ok(())
    }

    pub async fn delete(&self, person: &str, provider: &str) -> Result<()> {
        self.db
            .execute(
                &Query::new("DELETE FROM oauth_tokens WHERE person=?1 AND provider=?2")
                    .bind(person)
                    .bind(provider),
            )
            .await?;
        Ok(())
    }

    /// Row count, for diagnostics and tests.
    pub async fn count(&self) -> Result<i64> {
        let row = self.db.query_opt(&Query::new("SELECT COUNT(*) FROM oauth_tokens")).await?;
        match row {
            Some(r) => r.i64(0),
            None => Ok(0),
        }
    }

    /// The raw stored ciphertext, so a test can prove the token is not in clear.
    #[cfg(test)]
    async fn stored_ciphertext(&self, person: &str, provider: &str) -> Result<Vec<u8>> {
        let row = self
            .db
            .query_opt(
                &Query::new(
                    "SELECT token_enc FROM oauth_tokens WHERE person=?1 AND provider=?2",
                )
                .bind(person)
                .bind(provider),
            )
            .await?
            .expect("row present");
        row.blob(0)
    }

    /// Insert a row without going through encryption, so a test can plant one
    /// that cannot be decrypted.
    #[cfg(test)]
    async fn insert_raw(
        &self,
        person: &str,
        provider: &str,
        token_enc: Vec<u8>,
        expires_at: &str,
    ) -> Result<()> {
        self.db
            .execute(
                &Query::new(
                    "INSERT INTO oauth_tokens \
                     (person, provider, token_enc, scopes, expires_at, instance_url) \
                     VALUES (?1, ?2, ?3, 'repo', ?4, NULL)",
                )
                .bind(person)
                .bind(provider)
                .bind(token_enc)
                .bind(expires_at),
            )
            .await?;
        Ok(())
    }
}

/// Comma separated, empty entries dropped.
fn split_scopes(raw: &str) -> Vec<String> {
    raw.split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::rel::SqliteRelationalDb;

    fn key(b: u8) -> [u8; cipher::KEY_SIZE] {
        [b; cipher::KEY_SIZE]
    }

    async fn mem(k: [u8; cipher::KEY_SIZE]) -> RelationalOAuthPersistence {
        RelationalOAuthPersistence::open_in_memory(k).await.unwrap()
    }

    async fn on_disk(
        path: &std::path::Path,
        k: [u8; cipher::KEY_SIZE],
    ) -> RelationalOAuthPersistence {
        let db = Arc::new(SqliteRelationalDb::open(path).unwrap());
        RelationalOAuthPersistence::open(db, k).await.unwrap()
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

    #[tokio::test]
    async fn upsert_then_load() {
        let p = mem(key(1)).await;
        p.upsert("alice@test.com", "github", &session("gho_abc")).await.unwrap();

        let all = p.load_all().await.unwrap();
        assert_eq!(all.len(), 1);
        let (person, provider, s) = &all[0];
        assert_eq!(person, "alice@test.com");
        assert_eq!(provider, "github");
        assert_eq!(s.access_token, "gho_abc");
        assert_eq!(s.scopes, vec!["repo", "read:user"]);
        assert_eq!(s.expires_at.to_rfc3339(), "2030-01-01T00:00:00+00:00");
        assert_eq!(s.instance_url, None);
    }

    #[tokio::test]
    async fn upsert_replaces_the_row_for_the_same_key() {
        let p = mem(key(2)).await;
        p.upsert("bob@test.com", "gitlab", &session("first")).await.unwrap();
        let mut s = session("second");
        s.provider = "gitlab".into();
        s.scopes = vec!["api".into()];
        s.instance_url = Some("https://gitlab.example.test".into());
        p.upsert("bob@test.com", "gitlab", &s).await.unwrap();

        assert_eq!(p.count().await.unwrap(), 1, "primary key is (person, provider)");
        let all = p.load_all().await.unwrap();
        assert_eq!(all[0].2.access_token, "second");
        assert_eq!(all[0].2.scopes, vec!["api"]);
        assert_eq!(
            all[0].2.instance_url.as_deref(),
            Some("https://gitlab.example.test")
        );
    }

    #[tokio::test]
    async fn one_row_per_person_provider_pair() {
        let p = mem(key(3)).await;
        p.upsert("a@t.c", "github", &session("t1")).await.unwrap();
        p.upsert("a@t.c", "gitlab", &session("t2")).await.unwrap();
        p.upsert("b@t.c", "github", &session("t3")).await.unwrap();
        assert_eq!(p.count().await.unwrap(), 3);
        let all = p.load_all().await.unwrap();
        assert_eq!(all.len(), 3);
        // ordered by person then provider
        assert_eq!(all[0].1, "github");
        assert_eq!(all[1].1, "gitlab");
        assert_eq!(all[2].0, "b@t.c");
    }

    #[tokio::test]
    async fn delete_removes_one_row_and_is_idempotent() {
        let p = mem(key(4)).await;
        p.upsert("a@t.c", "github", &session("t1")).await.unwrap();
        p.upsert("a@t.c", "gitlab", &session("t2")).await.unwrap();
        p.delete("a@t.c", "github").await.unwrap();
        assert_eq!(p.count().await.unwrap(), 1);
        assert_eq!(p.load_all().await.unwrap()[0].1, "gitlab");
        p.delete("a@t.c", "github").await.unwrap();
        p.delete("nobody@t.c", "github").await.unwrap();
    }

    #[tokio::test]
    async fn token_is_not_stored_in_clear() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/oauth.db");
        let p = on_disk(&path, key(5)).await;
        p.upsert("a@t.c", "github", &session("gho_supersecret")).await.unwrap();

        let raw = p.stored_ciphertext("a@t.c", "github").await.unwrap();
        assert!(
            !raw.windows(15).any(|w| w == b"gho_supersecret"),
            "the token must be ciphertext on disk"
        );
        assert_eq!(raw.len(), cipher::NONCE_SIZE + cipher::TAG_SIZE + 15);
    }

    #[tokio::test]
    async fn rows_encrypted_with_another_key_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.db");
        {
            let old = on_disk(&path, key(6)).await;
            old.upsert("stale@t.c", "github", &session("old_token")).await.unwrap();
        }
        // key rotation: the old row can no longer be decrypted
        let new = on_disk(&path, key(7)).await;
        assert_eq!(new.count().await.unwrap(), 1, "the row is still there");
        assert!(
            new.load_all().await.unwrap().is_empty(),
            "but it is skipped instead of crashing startup"
        );

        // a new token under the new key loads fine alongside the unreadable one
        new.upsert("fresh@t.c", "github", &session("new_token")).await.unwrap();
        let all = new.load_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "fresh@t.c");
    }

    #[tokio::test]
    async fn corrupt_blobs_and_bad_timestamps_are_skipped() {
        let p = mem(key(8)).await;
        p.upsert("good@t.c", "github", &session("ok")).await.unwrap();
        p.insert_raw("corrupt@t.c", "github", vec![0u8; 40], "2030-01-01T00:00:00Z")
            .await
            .unwrap();
        p.insert_raw(
            "badtime@t.c",
            "github",
            cipher::encrypt(&key(8), "tok").unwrap(),
            "not-a-date",
        )
        .await
        .unwrap();

        assert_eq!(p.count().await.unwrap(), 3);
        let all = p.load_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "good@t.c");
    }

    #[tokio::test]
    async fn scopes_round_trip_including_empty() {
        let p = mem(key(9)).await;
        let mut s = session("t");
        s.scopes = Vec::new();
        p.upsert("a@t.c", "github", &s).await.unwrap();
        assert!(p.load_all().await.unwrap()[0].2.scopes.is_empty());

        assert_eq!(split_scopes(""), Vec::<String>::new());
        assert_eq!(split_scopes("a,,b"), vec!["a", "b"], "empty entries dropped");
    }

    #[tokio::test]
    async fn survives_reopen_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/oauth.db");
        {
            let p = on_disk(&path, key(10)).await;
            p.upsert("a@t.c", "github", &session("persisted")).await.unwrap();
        }
        assert!(path.exists());
        let p2 = on_disk(&path, key(10)).await;
        assert_eq!(p2.load_all().await.unwrap()[0].2.access_token, "persisted");
    }
}
