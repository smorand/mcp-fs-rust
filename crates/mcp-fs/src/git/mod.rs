//! Git subsystem: a bare repository per project whose objects live in the volume's
//! blob backend, with a SQLite index for refs and objects, served over the git
//! smart HTTP protocol.
//!
//! Port of the C# `src/McpFs/Git/*`. Physical layout per project:
//!
//! ```text
//! state/git/{project_id}.db        objects index, refs, remotes  (db)
//! state/git-repos/{project_id}/    bare libgit2 dir              (repo)
//! blob bucket, key git:{sha}       the git objects themselves    (odb)
//! state/oauth.db                   encrypted OAuth tokens        (oauth)
//! ```
//!
//! Wiring (the composition root owns this, see [`http::router`]):
//!
//! ```ignore
//! use mcp_fs::git;
//!
//! let git_store = git::GitRepoStore::shared(state.config.clone());
//! let tokens = git::OAuthTokenStore::from_env(&state.config)?;
//! if state.config.git.enabled {
//!     app = app.merge(git::http::router(state.clone(), git_store.clone()));
//! }
//! ```
//!
//! One documented deviation: `git2` cannot register a custom libgit2 ODB backend
//! from safe Rust, so instead of the C# `BlobBackedOdbBackend` the blob store is
//! synced to and from the on disk ODB around each libgit2 operation. The stored
//! bytes are identical. See [`odb`] for the full reasoning.

pub mod db;
pub mod http;
pub mod oauth;
pub mod odb;
pub mod repo;

pub use db::{GitObjectRow, GitRefRow, SqliteGitDb};
pub use oauth::{OAuthSession, OAuthTokenStore, SqliteOAuthPersistence};
pub use odb::{BlobObjectDb, blob_key, deserialize, serialize};
pub use repo::{GitRepoEntry, GitRepoStore};
