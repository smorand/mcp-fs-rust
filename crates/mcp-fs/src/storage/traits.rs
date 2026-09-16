//! Storage contracts. One-to-one port of the C# `Storage/Interfaces.cs`, so a
//! new backend is added by implementing a trait plus a branch in `backends.rs`.

use crate::errors::{Result, ToolError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// One entry in a volume's directory tree (file or directory).
/// Column names match the `nodes` table exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRow {
    pub path: String,
    pub parent: Option<String>,
    pub name: String,
    /// "dir" | "file"
    pub kind: String,
    pub size: i64,
    /// Full POSIX mode including type bits.
    pub mode: i64,
    pub mtime: f64,
    pub ctime: f64,
    pub atime: f64,
    pub sha256: Option<String>,
}

impl NodeRow {
    pub fn is_dir(&self) -> bool { self.kind == "dir" }
    pub fn is_file(&self) -> bool { self.kind == "file" }
}

/// Default POSIX modes, matching the C# implementation.
pub const MODE_DIR: i64 = 0o040_755;
pub const MODE_FILE: i64 = 0o100_644;

/// How much of a project's content is kept in the search index.
///
/// Stored on the project row, so it survives a restart and applies to every write
/// on the volume regardless of which surface performed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IndexMode {
    /// Nothing is indexed, and switching to it wipes what was indexed before.
    #[default]
    None,
    /// Full text only.
    Bm25,
    /// Vectors only.
    Rag,
    /// Full text and vectors.
    Both,
}

impl IndexMode {
    /// The stored form, which is also the wire form of the two admin tools.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bm25 => "bm25",
            Self::Rag => "rag",
            Self::Both => "both",
        }
    }

    /// True for every mode that indexes something.
    pub fn is_active(self) -> bool {
        self != Self::None
    }

    /// True when the mode needs an embedding endpoint to index anything.
    pub fn needs_embedding(self) -> bool {
        matches!(self, Self::Rag | Self::Both)
    }
}

impl fmt::Display for IndexMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for IndexMode {
    type Err = ToolError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "none" => Ok(Self::None),
            "bm25" => Ok(Self::Bm25),
            "rag" => Ok(Self::Rag),
            "both" => Ok(Self::Both),
            other => Err(ToolError::invalid_argument(format!(
                "unknown index mode '{other}', expected none, bm25, rag, or both"
            ))),
        }
    }
}

/// A project in the ACL registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub owner: String,
    pub created_at: String,
    /// Search index mode, `none` for every project that never set one.
    pub index_mode: IndexMode,
}

/// A project membership row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub project_id: String,
    pub person: String,
    /// "owner" | "member"
    pub role: String,
    pub added_by: String,
    pub added_at: String,
}

/// A volume's metadata tree plus content-addressed blob reference counts.
#[async_trait]
pub trait MetaBackend: Send + Sync {
    async fn get(&self, path: &str) -> Result<Option<NodeRow>>;
    async fn list_children(&self, parent: &str) -> Result<Vec<NodeRow>>;
    async fn subtree(&self, root: &str) -> Result<Vec<NodeRow>>;

    /// Upsert a file node. Returns a sha whose refcount hit 0 (caller GCs it).
    async fn put_file(
        &self,
        path: &str,
        sha256: Option<&str>,
        size: i64,
        mode: i64,
    ) -> Result<Option<String>>;

    /// Remove a file node. Returns a sha whose refcount hit 0.
    async fn delete_file(&self, path: &str) -> Result<Option<String>>;

    /// Remove a node and all descendants. Returns shas whose refcount hit 0.
    async fn remove_subtree(&self, path: &str) -> Result<Vec<String>>;

    async fn mkdirs(&self, path: &str, exist_ok: bool) -> Result<()>;
    async fn mkdir(&self, path: &str) -> Result<()>;
    async fn rmdir(&self, path: &str) -> Result<()>;
    async fn rename(&self, src: &str, dst: &str) -> Result<()>;
}

/// A content-addressed byte store, keyed by sha256, scoped to one volume.
#[async_trait]
pub trait BlobBackend: Send + Sync {
    async fn put(&self, sha256: &str, data: &[u8]) -> Result<()>;
    /// Read a blob, optionally a byte range.
    async fn get(&self, sha256: &str, offset: u64, length: Option<u64>) -> Result<Vec<u8>>;
    async fn exists(&self, sha256: &str) -> Result<bool>;
    async fn delete(&self, sha256: &str) -> Result<()>;
    async fn ensure_bucket(&self) -> Result<()>;
    async fn remove_bucket(&self) -> Result<()>;
}

/// ACL registry of projects and their members.
#[async_trait]
pub trait AdminBackend: Send + Sync {
    async fn connect(&self) -> Result<()>;
    async fn create_project(&self, project_id: &str, owner: &str) -> Result<Project>;
    async fn delete_project(&self, project_id: &str) -> Result<()>;
    async fn add_member(&self, project_id: &str, person: &str, added_by: &str) -> Result<Member>;
    async fn remove_member(&self, project_id: &str, person: &str) -> Result<()>;
    async fn get_project(&self, project_id: &str) -> Result<Option<Project>>;
    async fn list_projects_for(&self, person: &str) -> Result<Vec<Project>>;
    async fn list_all_projects(&self) -> Result<Vec<Project>>;
    async fn list_all_persons(&self) -> Result<Vec<String>>;
    async fn list_members(&self, project_id: &str) -> Result<Vec<Member>>;
    async fn is_member(&self, project_id: &str, person: &str) -> Result<bool>;

    /// `ERR_PROJECT_NOT_FOUND` when absent, `ERR_FORBIDDEN` when not a member.
    async fn require_member(&self, project_id: &str, person: &str) -> Result<()>;
    /// `ERR_PROJECT_NOT_FOUND` when absent, `ERR_FORBIDDEN` when not the owner.
    async fn require_owner(&self, project_id: &str, person: &str) -> Result<Project>;

    /// Store a project's search index mode. `ERR_PROJECT_NOT_FOUND` when absent.
    async fn set_index_mode(&self, project_id: &str, mode: IndexMode) -> Result<()>;
    /// Read a project's search index mode. `ERR_PROJECT_NOT_FOUND` when absent.
    async fn get_index_mode(&self, project_id: &str) -> Result<IndexMode>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_kind_helpers() {
        let f = NodeRow {
            path: "/a.txt".into(),
            parent: Some("/".into()),
            name: "a.txt".into(),
            kind: "file".into(),
            size: 3,
            mode: MODE_FILE,
            mtime: 1.0,
            ctime: 1.0,
            atime: 1.0,
            sha256: Some("abc".into()),
        };
        assert!(f.is_file() && !f.is_dir());

        let d = NodeRow { kind: "dir".into(), sha256: None, size: 0, mode: MODE_DIR, ..f.clone() };
        assert!(d.is_dir() && !d.is_file());
    }

    #[test]
    fn index_mode_round_trips_through_its_stored_form() {
        for mode in [IndexMode::None, IndexMode::Bm25, IndexMode::Rag, IndexMode::Both] {
            assert_eq!(mode.as_str().parse::<IndexMode>().expect("parses"), mode);
            assert_eq!(mode.to_string(), mode.as_str());
        }
        assert_eq!(IndexMode::default(), IndexMode::None);
    }

    /// The stored form is also the wire form of the two admin tools, so the serde
    /// rename must stay lowercase.
    #[test]
    fn index_mode_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&IndexMode::Bm25).expect("serializes"), "\"bm25\"");
        assert_eq!(
            serde_json::from_str::<IndexMode>("\"both\"").expect("deserializes"),
            IndexMode::Both
        );
    }

    #[test]
    fn an_unknown_index_mode_is_an_invalid_argument() {
        let e = "fancy".parse::<IndexMode>().expect_err("must be rejected");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("unknown index mode 'fancy'"), "{}", e.message);
    }

    #[test]
    fn active_and_embedding_predicates() {
        assert!(!IndexMode::None.is_active());
        assert!(IndexMode::Bm25.is_active());
        assert!(!IndexMode::Bm25.needs_embedding());
        assert!(IndexMode::Rag.needs_embedding());
        assert!(IndexMode::Both.needs_embedding());
    }

    #[test]
    fn default_modes_are_posix() {
        // 0o040755 = directory + rwxr-xr-x, 0o100644 = regular file + rw-r--r--
        assert_eq!(MODE_DIR, 16877);
        assert_eq!(MODE_FILE, 33188);
    }
}
