//! Additive, not-yet-routed rmcp server: one `#[tool]` method per `fs.*` tool.
//!
//! This is step one of SPEC-0013 (US-0003): every method below calls the exact
//! same `core::fs_ops` function, in the exact same order (authorize, normalize,
//! call the engine), that the equivalent handler in `crate::tools::{read,write,
//! edit,search,listing,metadata,lifecycle,document}` registers today through the
//! old hand-rolled `ToolRegistry`. No operation logic is reimplemented here: this
//! module only owns the rmcp dispatch wrapper (`#[tool_router]` / `#[tool]`).
//!
//! Nothing in this file is wired into any route yet (that is US-0008); it is
//! dead code until then, hence the module-level allow.

#![allow(dead_code)]

use std::sync::Arc;

use rmcp::ErrorData;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::fs_ops;
use crate::errors::ToolError;
use crate::git::GitRepoStore;
use crate::search::fusion::rrf_merge;
use crate::search::indexer;
use crate::state::AppState;
use crate::storage::VolumeClient;
use crate::storage::admin::validate_project_id;
use crate::storage::traits::IndexMode;
use crate::tools::registry_support::ToolCtx;

/// Server name advertised by `initialize`, matching the old hand-rolled transport.
const SERVER_NAME: &str = "mcp-fs";
use crate::util::normalize_identity;
use std::str::FromStr;

/// Turn a tool's `Result<Value, ToolError>` into the two MCP outcomes, with the
/// exact same error text the old SSE transport renders
/// (`An error occurred invoking '<tool>': <CODE>: <message>`).
fn to_call_result(
    tool: &str,
    result: Result<Value, ToolError>,
) -> Result<CallToolResult, ErrorData> {
    match result {
        Ok(v) => Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string(&v).expect("tool result must serialize"),
        )])),
        Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
            "An error occurred invoking '{tool}': {e}"
        ))])),
    }
}

/// Per-connection MCP server: the identity (`person`) is fixed for the lifetime
/// of this instance, exactly like `ToolCtx` is fixed for the lifetime of one
/// tool call today.
#[derive(Clone)]
pub struct McpServer {
    state: Arc<AppState>,
    person: String,
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    pub fn new(state: Arc<AppState>, person: String) -> Self {
        #[allow(unused_mut)]
        let mut tool_router = Self::tool_router();
        // Test-only (US-0008, DT-004): the extra router lives in its own
        // `#[cfg(test)]`-gated `impl` block below, so the whole item (macro
        // expansion included) is absent from a non-test build, not merely
        // the generated tool call branch.
        #[cfg(test)]
        {
            tool_router += Self::test_tool_router();
        }
        Self { state, person, tool_router }
    }

    /// Per-call `git.*` context: the `(person, state)` pair handed directly to
    /// the extracted `crate::tools::git`/`git_auth`/`git_pr` functions (US-0011).
    fn git_ctx(&self) -> ToolCtx {
        ToolCtx { person: self.person.clone(), state: self.state.clone() }
    }

    /// Port of `crate::tools::volume`: authorize, then open the volume.
    async fn volume(&self, mount_id: &str) -> Result<Arc<VolumeClient>, ToolError> {
        self.state.authorize(mount_id, &self.person).await?;
        self.state.stores.client(mount_id).await
    }

    /// Port of `crate::tools::authorize_only`.
    async fn authorize_only(&self, mount_id: &str) -> Result<(), ToolError> {
        self.state.authorize(mount_id, &self.person).await
    }

    /// Port of `crate::tools::norm`.
    fn norm(&self, raw: &str) -> Result<String, ToolError> {
        self.state.safety.normalize_path(raw)
    }
}

// ── parameter structs, one per tool, field order/defaults per TOOL_CONTRACT.txt ──

fn def_true() -> bool {
    true
}
fn def_false() -> bool {
    false
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume, e.g. /src/app.py.
    pub path: String,
    /// 0-based line offset to start reading from.
    #[serde(default)]
    pub offset_lines: i64,
    /// Maximum number of lines to return.
    #[serde(default = "def_limit_lines")]
    pub limit_lines: i64,
    /// Prefix each line with its 1-based line number.
    #[serde(default = "def_true")]
    pub line_numbered: bool,
}
fn def_limit_lines() -> i64 {
    2000
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadBytesArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// 0-based byte offset to start reading from.
    #[serde(default)]
    pub offset_bytes: i64,
    /// Maximum number of bytes to return.
    #[serde(default = "def_length_bytes")]
    pub length_bytes: i64,
}
fn def_length_bytes() -> i64 {
    65536
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadLinesArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// First 1-based line to return (inclusive).
    pub start_line: i64,
    /// Last 1-based line to return (inclusive).
    pub end_line: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadSectionArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// 1-based line whose indentation block is returned.
    pub anchor_line: i64,
    /// Maximum number of lines to return for the block.
    #[serde(default = "def_max_lines_200")]
    pub max_lines: i64,
}
fn def_max_lines_200() -> i64 {
    200
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadManyArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX paths to read, one entry per file.
    pub paths: Vec<String>,
    /// Maximum number of lines returned per file.
    #[serde(default = "def_per_file_cap_lines")]
    pub per_file_cap_lines: i64,
}
fn def_per_file_cap_lines() -> i64 {
    500
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HeadTailArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// Number of lines to return.
    #[serde(default = "def_lines_20")]
    pub lines: i64,
}
fn def_lines_20() -> i64 {
    20
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathOnlyArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume, e.g. /src/app.py.
    pub path: String,
    /// Full text content to write to the file.
    pub content: String,
    /// Allow overwriting an existing file (default no-clobber).
    #[serde(default)]
    pub overwrite: bool,
    /// Create missing parent directories.
    #[serde(default = "def_true")]
    pub create_parents: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppendArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// Text content to append at the end of the file.
    pub content: String,
    /// Create the file if it does not exist.
    #[serde(default)]
    pub create: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateEmptyArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path of the file to create.
    pub path: String,
    /// Succeed silently if the file already exists.
    #[serde(default)]
    pub exist_ok: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteBytesArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// File content, base64 encoded.
    pub base64: String,
    /// Allow overwriting an existing file (default no-clobber).
    #[serde(default)]
    pub overwrite: bool,
    /// Create missing parent directories.
    #[serde(default = "def_true")]
    pub create_parents: bool,
    /// Also generate the Markdown companion (path.md) through the configured document service.
    /// Supported for PowerPoint, Word, PDF, audio and video only.
    #[serde(default)]
    pub trigger_documentation_service: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// Exact text to find; must be unique unless replace_all is set.
    pub old_string: String,
    /// Replacement text substituted for old_string.
    pub new_string: String,
    /// Replace every occurrence instead of requiring a unique match.
    #[serde(default)]
    pub replace_all: bool,
    /// Return the diff without writing changes.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MultiEditArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// Ordered edits (old_string, new_string, replace_all) applied atomically.
    #[schemars(schema_with = "multi_edit_edits_schema")]
    pub edits: Value,
    /// Return the diff without writing changes.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchReplaceArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// Multi-line block of text to locate.
    pub search_block: String,
    /// Multi-line block that replaces search_block.
    pub replace_block: String,
    /// Allow whitespace tolerant (fuzzy) matching of search_block.
    #[serde(default)]
    pub fuzzy: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InsertAtLineArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// 1-based line number to insert content before.
    pub line: i64,
    /// Text content to insert.
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApplyPatchArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Multi-file V4A patch text to apply within the volume.
    pub patch_text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GlobArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Glob pattern to match file paths, e.g. **/*.cs.
    pub pattern: String,
    /// Absolute POSIX directory to search under.
    #[serde(default = "def_root")]
    pub root: String,
    /// Glob patterns whose matches are excluded from results.
    #[serde(default)]
    pub exclude_patterns: ExcludePatterns,
}
fn def_root() -> String {
    "/".to_string()
}

/// Hand-written to match the frozen contract exactly: an array of edit specs,
/// each with no `required` list, matching `inputSchema` round-tripped from a
/// JSON-typed `edits` field rather than a strongly-typed one.
/// A glob exclusion list whose frozen contract schema carries no `type` or
/// `items`, only `default: null`: the old hand-written contract never
/// constrained this field's shape, and the real default (empty, not null) is
/// deliberately NOT what the schema advertises. A custom `Serialize` that
/// always writes `null` is how `schemars`' struct-level default insertion is
/// made to reproduce exactly that, while `Deserialize` still defaults a
/// missing or null field to an empty list for real use.
#[derive(Debug, Default, Deserialize)]
pub struct ExcludePatterns(#[serde(deserialize_with = "deserialize_opt_vec")] pub Vec<String>);

fn deserialize_opt_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

impl Serialize for ExcludePatterns {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_none()
    }
}

impl JsonSchema for ExcludePatterns {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ExcludePatterns".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::Schema::from(serde_json::Map::new())
    }
}

fn multi_edit_edits_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::Schema::try_from(serde_json::json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "old_string": {"type": "string"},
                "new_string": {"type": "string"},
                "replace_all": {"type": "boolean"}
            }
        }
    }))
    .expect("valid literal schema")
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrepArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Search pattern; regex or literal depending on regex.
    pub pattern: String,
    /// Absolute POSIX directory to search under.
    #[serde(default = "def_root")]
    pub root: String,
    /// Glob limiting which files are searched.
    #[serde(default)]
    pub include_glob: Option<String>,
    /// Glob excluding files from the search.
    #[serde(default)]
    pub exclude_glob: Option<String>,
    /// Treat pattern as a regex when true, else a literal string.
    #[serde(default = "def_true")]
    pub regex: bool,
    /// Match case sensitively.
    #[serde(default = "def_true")]
    pub case_sensitive: bool,
    /// Output mode: files, content, or count.
    #[serde(default = "def_content")]
    pub output_mode: String,
    /// Lines of context around each match (content mode).
    #[serde(default)]
    pub context_lines: i64,
    /// Maximum number of matches to return.
    #[serde(default = "def_max_matches")]
    pub max_matches: i64,
}
fn def_content() -> String {
    "content".to_string()
}
fn def_max_matches() -> i64 {
    100
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindDefinitionArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Symbol name to locate the definition of.
    pub name: String,
    /// Absolute POSIX directory to search under.
    #[serde(default = "def_root")]
    pub root: String,
    /// Optional symbol kind filter, e.g. function, class, method.
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindReferencesArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Identifier name to find references to.
    pub name: String,
    /// Absolute POSIX directory to search under.
    #[serde(default = "def_root")]
    pub root: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListDirArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX directory to list.
    #[serde(default = "def_root")]
    pub path: String,
    /// Include dotfiles (names starting with a period).
    #[serde(default)]
    pub include_hidden: bool,
    /// Sort order: name or size.
    #[serde(default = "def_name")]
    pub sort_by: String,
    /// Include size and mtime for each entry.
    #[serde(default)]
    pub with_sizes: bool,
}
fn def_name() -> String {
    "name".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TreeArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX directory to walk from.
    #[serde(default = "def_root")]
    pub path: String,
    /// Maximum recursion depth to descend.
    #[serde(default = "def_max_depth")]
    pub max_depth: i64,
    /// Glob patterns whose matches are pruned from the tree.
    #[serde(default)]
    pub exclude_patterns: ExcludePatterns,
    /// Include size for each file node.
    #[serde(default)]
    pub with_sizes: bool,
}
fn def_max_depth() -> i64 {
    3
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HashArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path within the volume.
    pub path: String,
    /// Hash algorithm: md5, sha1, sha256, or sha512.
    #[serde(default = "def_sha256")]
    pub algo: String,
}
fn def_sha256() -> String {
    "sha256".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MkdirArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path of the directory to create.
    pub path: String,
    /// Create missing parent directories.
    #[serde(default = "def_true")]
    pub parents: bool,
    /// Succeed silently if the directory already exists.
    #[serde(default = "def_true")]
    pub exist_ok: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path to delete.
    pub path: String,
    /// Required to delete a non-empty directory.
    #[serde(default)]
    pub recursive: bool,
    /// Move to trash instead of hard deleting.
    #[serde(default = "def_true")]
    pub trash: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX source path to move.
    pub source: String,
    /// Absolute POSIX destination path.
    pub destination: String,
    /// Allow overwriting an existing destination (default no-clobber).
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CopyArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX source path to copy.
    pub source: String,
    /// Absolute POSIX destination path.
    pub destination: String,
    /// Allow overwriting an existing destination (default no-clobber).
    #[serde(default)]
    pub overwrite: bool,
    /// Required to copy a directory tree.
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MountOnlyArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
}

// ── git.auth*/git.pr_* parameter structs ──────────────────────────────

fn def_pr_state() -> String {
    "open".to_string()
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitAuthArgs {
    /// OAuth provider, github or gitlab.
    pub provider: String,
    /// Host to authenticate against; defaults to the provider's public host.
    #[serde(default)]
    pub host: Option<String>,
    /// Base URL of a self-hosted GitLab instance.
    #[serde(default)]
    pub instance_url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitAuthRevokeArgs {
    /// OAuth provider, github or gitlab.
    #[serde(default)]
    pub provider: Option<String>,
    /// Host to revoke the token for.
    #[serde(default)]
    pub host: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitAuthStatusArgs {
    /// OAuth provider, github or gitlab.
    #[serde(default)]
    pub provider: Option<String>,
    /// Host to check the status for.
    #[serde(default)]
    pub host: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitTokenSetArgs {
    /// Host the token belongs to, as declared in git.hosts.
    pub host: String,
    /// Personal access token value. Never echoed back.
    pub token: String,
    /// Expiry timestamp for the token, if known.
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitPrCreateArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Base branch the pull request merges into.
    pub base: String,
    /// Head branch carrying the change.
    pub head: String,
    /// Pull request title.
    pub title: String,
    /// Pull request body/description.
    #[serde(default)]
    pub body: Option<String>,
    /// Open the pull request as a draft.
    #[serde(default)]
    pub draft: bool,
    /// Name of the declared remote the pull request targets.
    #[serde(default = "def_origin")]
    pub remote: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitPrDiffArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Pull request number.
    pub pr_number: i64,
    /// Name of the declared remote the pull request belongs to.
    #[serde(default = "def_origin")]
    pub remote: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitPrGetArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Pull request number.
    pub pr_number: i64,
    /// Name of the declared remote the pull request belongs to.
    #[serde(default = "def_origin")]
    pub remote: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitPrListArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Filter: open, closed, merged, or all.
    #[serde(default = "def_pr_state")]
    pub state: String,
    /// Name of the declared remote to list pull requests for.
    #[serde(default = "def_origin")]
    pub remote: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitPrMergeArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Pull request number.
    pub pr_number: i64,
    /// Merge strategy: merge, squash, or rebase.
    pub strategy: String,
    /// Commit title for the merge, when the provider supports overriding it.
    #[serde(default)]
    pub commit_title: Option<String>,
    /// Commit message for the merge, when the provider supports overriding it.
    #[serde(default)]
    pub commit_message: Option<String>,
    /// Name of the declared remote the pull request belongs to.
    #[serde(default = "def_origin")]
    pub remote: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitPrReviewArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Pull request number.
    pub pr_number: i64,
    /// Review verdict: approve, request_changes, or comment.
    pub verdict: String,
    /// Review body; required for request_changes and comment.
    #[serde(default)]
    pub body: Option<String>,
    /// Name of the declared remote the pull request belongs to.
    #[serde(default = "def_origin")]
    pub remote: String,
}

// ── git.* parameter structs ──────────────────────────────────────────

fn def_origin() -> String {
    "origin".to_string()
}
fn def_mainline0() -> i64 {
    0
}
fn def_log_limit() -> i64 {
    20
}
fn def_depth0() -> i64 {
    0
}

/// One conflict resolution, round-tripped untouched into the old handler's
/// `resolutions` argument: `path` plus exactly one of `strategy` or `content`.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ResolutionArg {
    pub path: String,
    pub strategy: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RebaseTodoItem {
    pub action: String,
    pub sha: String,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitBlameArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path of the file to blame.
    pub path: String,
    /// Ref or commit to blame from; defaults to HEAD.
    #[serde(default)]
    pub ref_name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitBranchCreateArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Name of the branch to create, without the refs/heads/ prefix.
    pub name: String,
    /// Ref name, branch, tag or commit sha the new branch starts at; defaults to the currently
    /// checked-out commit.
    #[serde(default)]
    pub start_point: String,
    /// Check the new branch out, moving HEAD onto it and updating the volume's files to match;
    /// false leaves HEAD and every file untouched.
    #[serde(default)]
    pub checkout: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitBranchDeleteArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Name of the branch to delete, without the refs/heads/ prefix.
    pub name: String,
    /// Delete the branch even when it holds commits reachable from no other ref, leaving them
    /// unreachable.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitBranchResetArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Name of the branch to move, without the refs/heads/ prefix.
    pub name: String,
    /// Ref name, branch, tag or commit sha the branch is moved to.
    pub target_commit: String,
    /// Move the branch even when the move is not a fast-forward, leaving the commits only the
    /// old tip reached orphaned.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitBranchSwitchArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Name of the branch to switch to, without the refs/heads/ prefix.
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitCheckoutFileArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Commit SHA to restore the file from.
    pub commit_sha: String,
    /// Absolute POSIX path of the file to restore into the volume.
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitCherryPickArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Commit SHA to cherry-pick.
    pub commit_sha: String,
    /// For a merge commit, the parent its change is taken relative to.
    #[serde(default = "def_mainline0")]
    pub mainline: i64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitResolutionsArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// One resolution per conflicting path.
    #[serde(default)]
    pub resolutions: Option<Vec<ResolutionArg>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitCommitArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Commit message.
    pub message: String,
    /// Optional author name; defaults to the caller.
    #[serde(default)]
    pub author_name: Option<String>,
    /// Optional author email; defaults to the caller person id.
    #[serde(default)]
    pub author_email: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitDiffArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Base ref or commit to diff from.
    pub from_ref: String,
    /// Target ref or commit to diff to; omit to diff against the working tree.
    #[serde(default)]
    pub to_ref: Option<String>,
    /// Optional path filter limiting the diff.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitLogArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Ref, branch, tag, or commit to start from; defaults to HEAD.
    #[serde(default)]
    pub ref_name: Option<String>,
    /// Maximum number of commits to return.
    #[serde(default = "def_log_limit")]
    pub limit: i64,
    /// Optional path filter; only commits touching it are returned.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitMergeArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Branch, tag or commit to merge into the checked-out branch.
    pub source_ref: String,
    #[serde(default)]
    pub squash: bool,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitMergeResolveArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// One resolution per conflicting path.
    pub resolutions: Vec<ResolutionArg>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitRebaseArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Commit the checked-out branch's commits are replayed onto.
    pub onto: String,
    /// The explicit todo list, one entry per commit of the range.
    pub todo: Vec<RebaseTodoItem>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitRemoteAddArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Name of the remote, for example origin or upstream.
    pub name: String,
    /// HTTPS URL of the remote repository, without embedded credentials.
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitRemoteCloneArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// HTTPS URL of the repository to clone.
    pub url: String,
    /// Branch to check out after cloning; defaults to the remote's default branch.
    #[serde(default)]
    pub branch: Option<String>,
    /// Shallow clone depth; 0 for a full clone.
    #[serde(default = "def_depth0")]
    pub depth: i64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitRemoteFetchArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    #[serde(default = "def_origin")]
    pub remote: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitRemotePullArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Branch to pull; must be the one currently checked out.
    pub branch: String,
    #[serde(default = "def_origin")]
    pub remote: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitRemotePushArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Local branch to push.
    pub branch: String,
    #[serde(default = "def_origin")]
    pub remote: String,
    /// Name the branch is pushed as on the remote; defaults to `branch`.
    #[serde(default)]
    pub remote_branch: Option<String>,
    #[serde(default)]
    pub force: bool,
    /// Required with force: the remote sha believed to be overwritten.
    #[serde(default)]
    pub expected_remote_sha: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitRemoteRemoveArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Name of the remote to delete; matched case-sensitively.
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitResetArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Ref name, branch, tag or commit sha the current branch is moved to.
    pub target_ref: String,
    /// 'soft' to move the pointer only, or 'hard' to also rewrite the volume.
    pub mode: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitRevertArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Commit SHA to revert.
    pub commit_sha: String,
    /// For a merge commit, the parent the revert is computed against.
    #[serde(default = "def_mainline0")]
    pub mainline: i64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitShowArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Commit SHA to show details and diff for.
    pub commit_sha: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitStashIdArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    pub stash_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitStashSaveArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AuditLogArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Only return entries at or after this Unix timestamp (seconds).
    #[serde(default)]
    pub since: Option<f64>,
    /// Maximum number of recent entries to return.
    #[serde(default = "def_lines_20")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractTextArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path of the source document.
    pub path: String,
    /// Maximum characters of Markdown to store.
    #[serde(default = "def_max_chars")]
    pub max_chars: i64,
    /// Number of leading characters returned as a preview.
    #[serde(default = "def_preview_chars")]
    pub preview_chars: i64,
    /// Enable OCR for images via a configured multimodal provider.
    #[serde(default = "def_true")]
    pub ocr: bool,
    /// Force re-extraction even if the companion .md is up to date.
    #[serde(default)]
    pub refresh: bool,
}
fn def_max_chars() -> i64 {
    200_000
}
fn def_preview_chars() -> i64 {
    4_000
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteDocxArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path of the .docx file to write.
    pub path: String,
    /// Markdown source rendered into the Word document.
    pub markdown: String,
    /// Optional document title.
    #[serde(default)]
    pub title: Option<String>,
    /// Allow overwriting an existing file (default no-clobber).
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DocumentizeArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path of the stored document to convert. PowerPoint, Word, PDF, audio and video only.
    pub path: String,
    /// Allow overwriting an existing companion .md (default no-clobber).
    #[serde(default)]
    pub overwrite: bool,
}

// ── admin.* parameter structs ────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateProjectArgs {
    /// New project id: 3 to 32 chars, lowercase letters, digits, hyphens, alphanumeric bounds.
    pub project_id: String,
    /// Person id who owns the new project.
    pub owner: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectIdArgs {
    /// Id of the project.
    pub project_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectPersonArgs {
    /// Id of the project.
    pub project_id: String,
    /// Person id.
    pub person: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetIndexModeArgs {
    /// Id of the project whose search index mode is set.
    pub project_id: String,
    /// New index mode: none, bm25, rag, or both.
    pub mode: String,
}

// ── search.* parameter structs ───────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchIndexArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path to index (file or directory).
    pub path: String,
    /// Recurse into subdirectories when path is a directory.
    #[serde(default)]
    pub recursive: bool,
    /// Maximum character size of each indexed chunk.
    #[serde(default = "def_chunk_size")]
    pub chunk_size: i64,
    /// Character overlap between consecutive chunks.
    #[serde(default = "def_chunk_overlap")]
    pub chunk_overlap: i64,
}
fn def_chunk_size() -> i64 {
    1000
}
fn def_chunk_overlap() -> i64 {
    100
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchQueryArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Search query text.
    pub query: String,
    /// Query mode: bm25, rag, or both. Defaults to server config.
    pub mode: Option<String>,
    /// Maximum number of results to return.
    #[serde(default = "def_top_k")]
    pub top_k: i64,
    /// Apply reranking when configured and available.
    #[serde(default = "def_true")]
    pub rerank: bool,
}
fn def_top_k() -> i64 {
    10
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchDeleteArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Absolute POSIX path to remove from the index.
    pub path: String,
    /// Recurse into subdirectories when path is a directory.
    #[serde(default)]
    pub recursive: bool,
}

/// `fs.multi_edit`'s `edits` array as raw JSON, same shape the old handler reads.
fn edit_specs(v: &Value) -> Result<Vec<Value>, ToolError> {
    match v {
        Value::Array(a) => Ok(a.clone()),
        Value::Null => Err(ToolError::invalid_argument("missing required argument 'edits'")),
        _ => Err(ToolError::invalid_argument("argument 'edits' must be an array of objects")),
    }
}

const EXTRACT_DESC: &str = "Extract a document to Markdown and store it as a companion .md next to the source \
(report.pdf -> report.md), reusing it if already up to date. Returns md_path + a preview; \
read the .md with fs.read for the full content. Handles PDF, DOCX, PPTX, XLSX, HTML, CSV, \
images (OCR via a configured multimodal provider) and text; audio/video unsupported.";

// `vis = "pub"` (US-0013): the REST `/api/swagger.json` catalog (`api/openapi.rs`)
// needs the 94 contract tool schemas with no live session, and this static,
// side-effect-free accessor is the cheapest way to get them.
#[tool_router(router = tool_router, vis = "pub")]
impl McpServer {
    // ── read family ──────────────────────────────────────────────────────────

    #[tool(name = "fs.read", description = "Read a text file with line-numbered, paged output.")]
    async fn fs_read(
        &self,
        Parameters(a): Parameters<ReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::read_window(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                a.offset_lines,
                a.limit_lines,
                a.line_numbered,
            )
            .await
        }
        .await;
        to_call_result("fs.read", out)
    }

    #[tool(name = "fs.read_bytes", description = "Read raw bytes (base64) with MIME type.")]
    async fn fs_read_bytes(
        &self,
        Parameters(a): Parameters<ReadBytesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::read_bytes_b64(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                a.offset_bytes,
                a.length_bytes,
            )
            .await
        }
        .await;
        to_call_result("fs.read_bytes", out)
    }

    #[tool(
        name = "fs.read_lines",
        description = "Read an inclusive line range [start_line, end_line]."
    )]
    async fn fs_read_lines(
        &self,
        Parameters(a): Parameters<ReadLinesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::read_lines(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                a.start_line,
                a.end_line,
            )
            .await
        }
        .await;
        to_call_result("fs.read_lines", out)
    }

    #[tool(
        name = "fs.read_section",
        description = "Read the indentation block around an anchor line."
    )]
    async fn fs_read_section(
        &self,
        Parameters(a): Parameters<ReadSectionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::read_section(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                a.anchor_line,
                a.max_lines,
            )
            .await
        }
        .await;
        to_call_result("fs.read_section", out)
    }

    #[tool(
        name = "fs.read_many",
        description = "Batch read several files with per-file error isolation."
    )]
    async fn fs_read_many(
        &self,
        Parameters(a): Parameters<ReadManyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            fs_ops::read_many(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &a.paths,
                a.per_file_cap_lines,
            )
            .await
        }
        .await;
        to_call_result("fs.read_many", out)
    }

    #[tool(name = "fs.head", description = "First N lines of a file.")]
    async fn fs_head(
        &self,
        Parameters(a): Parameters<HeadTailArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::head(&client, &self.state.safety, &self.person, &a.mount_id, &path, a.lines)
                .await
        }
        .await;
        to_call_result("fs.head", out)
    }

    #[tool(name = "fs.tail", description = "Last N lines of a file.")]
    async fn fs_tail(
        &self,
        Parameters(a): Parameters<HeadTailArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::tail(&client, &self.state.safety, &self.person, &a.mount_id, &path, a.lines)
                .await
        }
        .await;
        to_call_result("fs.tail", out)
    }

    #[tool(name = "fs.count_lines", description = "Count lines without returning content.")]
    async fn fs_count_lines(
        &self,
        Parameters(a): Parameters<PathOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::count_lines(&client, &path).await
        }
        .await;
        to_call_result("fs.count_lines", out)
    }

    // ── write family ─────────────────────────────────────────────────────────

    #[tool(
        name = "fs.write",
        description = "Create or overwrite a file (no-clobber by default, atomic)."
    )]
    async fn fs_write(
        &self,
        Parameters(a): Parameters<WriteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let res = fs_ops::write_text(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                &a.content,
                a.overwrite,
                a.create_parents,
            )
            .await?;
            indexer::after_write(&self.state, &a.mount_id, &path, &a.content).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.write", out)
    }

    #[tool(name = "fs.append", description = "Append content to a file (optionally create it).")]
    async fn fs_append(
        &self,
        Parameters(a): Parameters<AppendArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let res = fs_ops::append_text(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                &a.content,
                a.create,
            )
            .await?;
            indexer::after_write_reread(&self.state, &a.mount_id, &path, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.append", out)
    }

    #[tool(name = "fs.create_empty", description = "Create an empty file (touch).")]
    async fn fs_create_empty(
        &self,
        Parameters(a): Parameters<CreateEmptyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::create_empty(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                a.exist_ok,
            )
            .await
        }
        .await;
        to_call_result("fs.create_empty", out)
    }

    #[tool(name = "fs.write_bytes", description = "Write raw bytes (base64) to a file.")]
    async fn fs_write_bytes(
        &self,
        Parameters(a): Parameters<WriteBytesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            use base64::Engine as _;
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let data =
                base64::engine::general_purpose::STANDARD.decode(a.base64.trim()).map_err(|e| {
                    ToolError::invalid_argument(format!(
                        "argument 'base64' is not valid base64: {e}"
                    ))
                })?;
            let res = fs_ops::write_bytes_documented(
                &client,
                &self.state.safety,
                self.state.doc_service.as_deref(),
                &self.person,
                &a.mount_id,
                &path,
                &data,
                a.overwrite,
                a.create_parents,
                a.trigger_documentation_service,
            )
            .await?;
            indexer::after_write_reread(&self.state, &a.mount_id, &path, &client).await;
            indexer::after_companion(&self.state, &a.mount_id, &res, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.write_bytes", out)
    }

    // ── edit family ──────────────────────────────────────────────────────────

    #[tool(name = "fs.edit", description = "Replace a unique string; dry_run returns the diff.")]
    async fn fs_edit(
        &self,
        Parameters(a): Parameters<EditArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let res = fs_ops::edit_unique(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                &a.old_string,
                &a.new_string,
                a.replace_all,
                a.dry_run,
            )
            .await?;
            if !a.dry_run {
                indexer::after_write_reread(&self.state, &a.mount_id, &path, &client).await;
            }
            Ok(res)
        }
        .await;
        to_call_result("fs.edit", out)
    }

    #[tool(
        name = "fs.multi_edit",
        description = "Apply several edits atomically (all or nothing)."
    )]
    async fn fs_multi_edit(
        &self,
        Parameters(a): Parameters<MultiEditArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let edits = edit_specs(&a.edits)?;
            let res = fs_ops::multi_edit(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                &edits,
                a.dry_run,
            )
            .await?;
            if !a.dry_run {
                indexer::after_write_reread(&self.state, &a.mount_id, &path, &client).await;
            }
            Ok(res)
        }
        .await;
        to_call_result("fs.multi_edit", out)
    }

    #[tool(
        name = "fs.search_replace",
        description = "Replace a multi-line block (optional fuzzy match)."
    )]
    async fn fs_search_replace(
        &self,
        Parameters(a): Parameters<SearchReplaceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let res = fs_ops::search_replace(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                &a.search_block,
                &a.replace_block,
                a.fuzzy,
            )
            .await?;
            indexer::after_write_reread(&self.state, &a.mount_id, &path, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.search_replace", out)
    }

    #[tool(
        name = "fs.insert_at_line",
        description = "Insert content before a 1-based line number."
    )]
    async fn fs_insert_at_line(
        &self,
        Parameters(a): Parameters<InsertAtLineArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let res = fs_ops::insert_at_line(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                a.line,
                &a.content,
            )
            .await?;
            indexer::after_write_reread(&self.state, &a.mount_id, &path, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.insert_at_line", out)
    }

    #[tool(
        name = "fs.apply_patch",
        description = "Apply a multi-file V4A patch within one volume."
    )]
    async fn fs_apply_patch(
        &self,
        Parameters(a): Parameters<ApplyPatchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let res = fs_ops::apply_patch(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &a.patch_text,
            )
            .await?;
            indexer::after_patch(&self.state, &a.mount_id, &res, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.apply_patch", out)
    }

    // ── search family ────────────────────────────────────────────────────────

    #[tool(name = "fs.glob", description = "Find files by glob pattern, newest first (cap 100).")]
    async fn fs_glob(
        &self,
        Parameters(a): Parameters<GlobArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let root = self.norm(&a.root)?;
            fs_ops::glob_files(&client, &root, &a.pattern, &a.exclude_patterns.0).await
        }
        .await;
        to_call_result("fs.glob", out)
    }

    #[tool(name = "fs.grep", description = "Search file contents (files|content|count modes).")]
    async fn fs_grep(
        &self,
        Parameters(a): Parameters<GrepArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let root = self.norm(&a.root)?;
            fs_ops::grep_files(
                &client,
                &root,
                &a.pattern,
                a.include_glob.as_deref(),
                a.exclude_glob.as_deref(),
                a.regex,
                a.case_sensitive,
                &a.output_mode,
                a.context_lines,
                a.max_matches,
            )
            .await
        }
        .await;
        to_call_result("fs.grep", out)
    }

    #[tool(name = "fs.find_definition", description = "Find a symbol definition (language-aware).")]
    async fn fs_find_definition(
        &self,
        Parameters(a): Parameters<FindDefinitionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let root = self.norm(&a.root)?;
            fs_ops::find_definitions(&client, &root, &a.name, a.kind.as_deref()).await
        }
        .await;
        to_call_result("fs.find_definition", out)
    }

    #[tool(
        name = "fs.find_references",
        description = "Find identifier references (language-aware)."
    )]
    async fn fs_find_references(
        &self,
        Parameters(a): Parameters<FindReferencesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let _root = self.norm(&a.root)?;
            fs_ops::find_references(&client, &_root, &a.name).await
        }
        .await;
        to_call_result("fs.find_references", out)
    }

    // ── listing family ───────────────────────────────────────────────────────

    #[tool(
        name = "fs.list_dir",
        description = "Flat directory listing with kinds and optional sizes."
    )]
    async fn fs_list_dir(
        &self,
        Parameters(a): Parameters<ListDirArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::list_dir(&client, &path, a.include_hidden, &a.sort_by, a.with_sizes).await
        }
        .await;
        to_call_result("fs.list_dir", out)
    }

    #[tool(name = "fs.tree", description = "Recursive JSON tree to a maximum depth.")]
    async fn fs_tree(
        &self,
        Parameters(a): Parameters<TreeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::tree(&client, &path, a.max_depth, &a.exclude_patterns.0, a.with_sizes).await
        }
        .await;
        to_call_result("fs.tree", out)
    }

    // ── metadata family ──────────────────────────────────────────────────────

    #[tool(name = "fs.stat", description = "POSIX metadata for a path.")]
    async fn fs_stat(
        &self,
        Parameters(a): Parameters<PathOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::stat_info(&client, &path).await
        }
        .await;
        to_call_result("fs.stat", out)
    }

    #[tool(name = "fs.exists", description = "Probe whether a path exists and its kind.")]
    async fn fs_exists(
        &self,
        Parameters(a): Parameters<PathOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::exists_info(&client, &path).await
        }
        .await;
        to_call_result("fs.exists", out)
    }

    #[tool(name = "fs.hash", description = "Content hash (md5|sha1|sha256|sha512).")]
    async fn fs_hash(
        &self,
        Parameters(a): Parameters<HashArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::hash_file(&client, &path, &a.algo).await
        }
        .await;
        to_call_result("fs.hash", out)
    }

    // ── lifecycle family ─────────────────────────────────────────────────────

    #[tool(name = "fs.mkdir", description = "Create a directory (parents by default).")]
    async fn fs_mkdir(
        &self,
        Parameters(a): Parameters<MkdirArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            fs_ops::mkdir(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                a.parents,
                a.exist_ok,
            )
            .await
        }
        .await;
        to_call_result("fs.mkdir", out)
    }

    #[tool(name = "fs.delete", description = "Delete a path (moves to trash by default).")]
    async fn fs_delete(
        &self,
        Parameters(a): Parameters<DeleteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let indexed = indexer::paths_under(&self.state, &a.mount_id, &path, &client).await;
            let res = fs_ops::delete_path(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                a.recursive,
                a.trash,
            )
            .await?;
            indexer::after_delete_many(&self.state, &a.mount_id, &indexed).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.delete", out)
    }

    #[tool(name = "fs.move", description = "Rename or relocate a path (no-clobber by default).")]
    async fn fs_move(
        &self,
        Parameters(a): Parameters<MoveArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let src = self.norm(&a.source)?;
            let dst = self.norm(&a.destination)?;
            let indexed =
                indexer::paths_displaced_by_move(&self.state, &a.mount_id, &src, &dst, &client)
                    .await;
            let res = fs_ops::move_path(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &src,
                &dst,
                a.overwrite,
            )
            .await?;
            indexer::after_move(&self.state, &a.mount_id, &indexed, &dst, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.move", out)
    }

    #[tool(name = "fs.copy", description = "Copy a file or tree (no-clobber by default).")]
    async fn fs_copy(
        &self,
        Parameters(a): Parameters<CopyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let src = self.norm(&a.source)?;
            let dst = self.norm(&a.destination)?;
            let res = fs_ops::copy_path(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &src,
                &dst,
                a.overwrite,
                a.recursive,
            )
            .await?;
            indexer::after_copy(&self.state, &a.mount_id, &dst, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.copy", out)
    }

    #[tool(
        name = "fs.list_allowed_roots",
        description = "List the volume roots the caller can access."
    )]
    async fn fs_list_allowed_roots(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.authorize_only(&a.mount_id).await?;
            let projects = self.state.admin.list_projects_for(&self.person).await?;
            let roots: Vec<Value> = projects
                .iter()
                .map(|p| serde_json::json!({"mount_id": p.id, "root": "/", "owner": p.owner}))
                .collect();
            Ok(serde_json::json!({"person": self.person, "roots": roots}))
        }
        .await;
        to_call_result("fs.list_allowed_roots", out)
    }

    #[tool(name = "fs.audit_log", description = "Recent mutations performed in this session.")]
    async fn fs_audit_log(
        &self,
        Parameters(a): Parameters<AuditLogArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.authorize_only(&a.mount_id).await?;
            let mut entries = self.state.safety.audit(&self.person, &a.mount_id);
            if let Some(since) = a.since {
                entries.retain(|e| e.timestamp >= since);
            }
            let skip = (entries.len() as i64 - a.limit).max(0) as usize;
            let recent: Vec<Value> = entries
                .iter()
                .skip(skip)
                .map(|e| {
                    serde_json::json!({
                        "timestamp": e.timestamp,
                        "op": e.op,
                        "path": e.path,
                        "detail": e.detail,
                    })
                })
                .collect();
            Ok(serde_json::json!({"entries": recent}))
        }
        .await;
        to_call_result("fs.audit_log", out)
    }

    // ── document family ──────────────────────────────────────────────────────

    #[tool(name = "fs.extract_text", description = EXTRACT_DESC)]
    async fn fs_extract_text(
        &self,
        Parameters(a): Parameters<ExtractTextArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let res = fs_ops::extract_document(
                &client,
                &self.state.safety,
                &self.state.config.extract.ocr,
                &self.person,
                &a.mount_id,
                &path,
                a.max_chars.max(0) as usize,
                a.preview_chars.max(0) as usize,
                a.ocr,
                a.refresh,
            )
            .await?;
            indexer::after_companion(&self.state, &a.mount_id, &res, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.extract_text", out)
    }

    #[tool(
        name = "fs.write_docx",
        description = "Render Markdown into a .docx Word document and write it to the volume."
    )]
    async fn fs_write_docx(
        &self,
        Parameters(a): Parameters<WriteDocxArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let res = fs_ops::write_docx(
                &client,
                &self.state.safety,
                &self.person,
                &a.mount_id,
                &path,
                &a.markdown,
                a.title.as_deref(),
                a.overwrite,
            )
            .await?;
            indexer::after_write_reread(&self.state, &a.mount_id, &path, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.write_docx", out)
    }

    #[tool(
        name = "fs.documentize",
        description = "Generate the Markdown companion of a stored document."
    )]
    async fn fs_documentize(
        &self,
        Parameters(a): Parameters<DocumentizeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let client = self.volume(&a.mount_id).await?;
            let path = self.norm(&a.path)?;
            let res = fs_ops::documentize(
                &client,
                &self.state.safety,
                self.state.doc_service.as_deref(),
                &self.person,
                &a.mount_id,
                &path,
                a.overwrite,
            )
            .await?;
            indexer::after_companion(&self.state, &a.mount_id, &res, &client).await;
            Ok(res)
        }
        .await;
        to_call_result("fs.documentize", out)
    }

    // ── admin.* family ────────────────────────────────────────────────

    #[tool(
        name = "admin.create_project",
        description = "Create a project for a designated owner and provision its volume (platform admin only)."
    )]
    async fn admin_create_project(
        &self,
        Parameters(a): Parameters<CreateProjectArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.require_admin(&self.person)?;
            validate_project_id(&a.project_id)?;
            if a.owner.trim().is_empty() {
                return Err(ToolError::invalid_argument("owner is required"));
            }
            let project = self.state.admin.create_project(&a.project_id, &a.owner).await?;
            if let Err(e) = self.state.stores.provision_volume(&a.project_id).await {
                let _ = self.state.admin.delete_project(&a.project_id).await;
                return Err(e);
            }
            Ok(serde_json::json!({
                "project_id": project.id,
                "owner": project.owner,
                "created_at": project.created_at,
            }))
        }
        .await;
        to_call_result("admin.create_project", out)
    }

    #[tool(
        name = "admin.delete_project",
        description = "Delete a project and recursively tear down its volume (owner or platform admin)."
    )]
    async fn admin_delete_project(
        &self,
        Parameters(a): Parameters<ProjectIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.require_owner_or_admin(&a.project_id, &self.person).await?;
            self.state.stores.teardown_volume(&a.project_id).await?;
            if self.state.config.git.enabled {
                let store = GitRepoStore::shared(
                    self.state.config.clone(),
                    self.state.stores.relational().clone(),
                );
                store.purge_repo(&a.project_id).await?;
            }
            self.state.admin.delete_project(&a.project_id).await?;
            Ok(serde_json::json!({"project_id": a.project_id, "deleted": true}))
        }
        .await;
        to_call_result("admin.delete_project", out)
    }

    #[tool(name = "admin.list_projects", description = "List projects the caller can access.")]
    async fn admin_list_projects(&self) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let person = normalize_identity(&self.person);
            let projects = self.state.admin.list_projects_for(&self.person).await?;
            let entries: Vec<Value> = projects
                .into_iter()
                .map(|p| {
                    serde_json::json!({
                        "project_id": p.id,
                        "owner": p.owner,
                        "created_at": p.created_at,
                        "index_mode": p.index_mode,
                        "is_owner": normalize_identity(&p.owner) == person,
                    })
                })
                .collect();
            Ok(serde_json::json!({"projects": entries}))
        }
        .await;
        to_call_result("admin.list_projects", out)
    }

    #[tool(
        name = "admin.list_all_projects",
        description = "List every project (platform admin only)."
    )]
    async fn admin_list_all_projects(&self) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.require_admin(&self.person)?;
            let projects = self.state.admin.list_all_projects().await?;
            let entries: Vec<Value> = projects
                .into_iter()
                .map(|p| {
                    serde_json::json!({
                        "project_id": p.id,
                        "owner": p.owner,
                        "created_at": p.created_at,
                        "index_mode": p.index_mode,
                    })
                })
                .collect();
            Ok(serde_json::json!({"projects": entries}))
        }
        .await;
        to_call_result("admin.list_all_projects", out)
    }

    #[tool(
        name = "admin.list_users",
        description = "List every known person and platform admins (platform admin only)."
    )]
    async fn admin_list_users(&self) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.require_admin(&self.person)?;
            let mut persons: std::collections::BTreeSet<String> =
                self.state.admin.list_all_persons().await?.into_iter().collect();
            for a in &self.state.config.auth.admins {
                persons.insert(a.clone());
            }
            let users: Vec<Value> = persons
                .into_iter()
                .map(|p| {
                    let is_admin = self.state.is_admin(&p);
                    serde_json::json!({"person": p, "is_admin": is_admin})
                })
                .collect();
            Ok(serde_json::json!({"users": users}))
        }
        .await;
        to_call_result("admin.list_users", out)
    }

    #[tool(
        name = "admin.add_member",
        description = "Add a person to a project (owner or platform admin)."
    )]
    async fn admin_add_member(
        &self,
        Parameters(a): Parameters<ProjectPersonArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.require_owner_or_admin(&a.project_id, &self.person).await?;
            let member =
                self.state.admin.add_member(&a.project_id, &a.person, &self.person).await?;
            Ok(serde_json::json!({
                "project_id": a.project_id,
                "person": member.person,
                "role": member.role,
            }))
        }
        .await;
        to_call_result("admin.add_member", out)
    }

    #[tool(
        name = "admin.remove_member",
        description = "Remove a person from a project (owner or platform admin)."
    )]
    async fn admin_remove_member(
        &self,
        Parameters(a): Parameters<ProjectPersonArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.require_owner_or_admin(&a.project_id, &self.person).await?;
            self.state.admin.remove_member(&a.project_id, &a.person).await?;
            Ok(serde_json::json!({
                "project_id": a.project_id,
                "person": a.person,
                "removed": true,
            }))
        }
        .await;
        to_call_result("admin.remove_member", out)
    }

    #[tool(
        name = "admin.list_members",
        description = "List members of a project (member or platform admin)."
    )]
    async fn admin_list_members(
        &self,
        Parameters(a): Parameters<ProjectIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            if self.state.is_admin(&self.person) {
                if self.state.admin.get_project(&a.project_id).await?.is_none() {
                    return Err(ToolError::project_not_found(&a.project_id));
                }
            } else {
                self.state.admin.require_member(&a.project_id, &self.person).await?;
            }
            let members = self.state.admin.list_members(&a.project_id).await?;
            let entries: Vec<Value> = members
                .into_iter()
                .map(|m| {
                    serde_json::json!({"person": m.person, "role": m.role, "added_by": m.added_by})
                })
                .collect();
            Ok(serde_json::json!({"project_id": a.project_id, "members": entries}))
        }
        .await;
        to_call_result("admin.list_members", out)
    }

    #[tool(
        name = "admin.set_index_mode",
        description = "Set the search index mode for a project (owner or platform admin). \
             none=no index, bm25=full-text only, rag=vector only, both=full-text and vector. \
             Switching to an active mode triggers an initial full index of existing files in \
             the background. Switching to none wipes the index immediately."
    )]
    async fn admin_set_index_mode(
        &self,
        Parameters(a): Parameters<SetIndexModeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.require_owner_or_admin(&a.project_id, &self.person).await?;
            let new_mode = IndexMode::from_str(&a.mode)?;

            let backend = self.state.search.as_ref().ok_or_else(|| {
                ToolError::not_supported(
                    "search is not enabled; set search.enabled: true in config",
                )
            })?;
            if new_mode.needs_embedding() && self.state.config.search.embedding.endpoint.is_empty()
            {
                return Err(ToolError::invalid_argument(format!(
                    "index mode '{new_mode}' requires search.embedding.endpoint to be configured"
                )));
            }

            let old_mode = self.state.admin.get_index_mode(&a.project_id).await?;
            self.state.admin.set_index_mode(&a.project_id, new_mode).await?;

            let client = self.state.stores.client(&a.project_id).await?;
            let reindex_started = crate::search::indexer::ProjectIndexer::new(backend)
                .on_mode_change(&a.project_id, old_mode, new_mode, client)
                .await?;

            Ok(serde_json::json!({
                "project_id": a.project_id,
                "index_mode": new_mode,
                "previous_mode": old_mode,
                "reindex_started": reindex_started,
            }))
        }
        .await;
        to_call_result("admin.set_index_mode", out)
    }

    #[tool(
        name = "admin.get_index_mode",
        description = "Get the current search index mode for a project (member or platform admin)."
    )]
    async fn admin_get_index_mode(
        &self,
        Parameters(a): Parameters<ProjectIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            if self.state.is_admin(&self.person) {
                if self.state.admin.get_project(&a.project_id).await?.is_none() {
                    return Err(ToolError::project_not_found(&a.project_id));
                }
            } else {
                self.state.admin.require_member(&a.project_id, &self.person).await?;
            }
            let mode = self.state.admin.get_index_mode(&a.project_id).await?;
            Ok(serde_json::json!({"project_id": a.project_id, "index_mode": mode}))
        }
        .await;
        to_call_result("admin.get_index_mode", out)
    }

    // ── search.* family ────────────────────────────────────────────

    #[tool(
        name = "search.index",
        description = "Index a file or directory into the search engine for this volume."
    )]
    async fn search_index(
        &self,
        Parameters(a): Parameters<SearchIndexArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.authorize(&a.mount_id, &self.person).await?;
            let path = self.norm(&a.path)?;
            let chunk_size = a.chunk_size as usize;
            let chunk_overlap = a.chunk_overlap as usize;

            let backend = self.state.search.as_ref().ok_or_else(|| {
                ToolError::not_supported(
                    "search is not enabled; set search.enabled: true in config",
                )
            })?;

            let client = self.state.stores.client(&a.mount_id).await?;

            let mut indexed = 0usize;
            let mut skipped = 0usize;

            let paths_to_index: Vec<String> = if a.recursive {
                fs_ops::iter_files(&client, &path, &[])
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(p, _mtime)| p)
                    .collect()
            } else {
                vec![path.clone()]
            };

            for file_path in &paths_to_index {
                match client.read_text(file_path).await {
                    Ok(text) => {
                        match backend
                            .index_path(&a.mount_id, file_path, &text, chunk_size, chunk_overlap)
                            .await
                        {
                            Ok(n) => indexed += n,
                            Err(_) => skipped += 1,
                        }
                    }
                    Err(_) => {
                        skipped += 1;
                    }
                }
            }

            Ok(serde_json::json!({
                "indexed": indexed,
                "skipped": skipped,
                "path": path,
            }))
        }
        .await;
        to_call_result("search.index", out)
    }

    #[tool(
        name = "search.query",
        description = "Search indexed content. mode overrides the server default (bm25, rag, both)."
    )]
    async fn search_query(
        &self,
        Parameters(a): Parameters<SearchQueryArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.authorize(&a.mount_id, &self.person).await?;

            let top_k = a.top_k as usize;
            let _rerank = a.rerank;

            let config_mode = &self.state.config.search.mode;
            let mode = a.mode.clone().unwrap_or_else(|| config_mode.clone());

            let backend = self.state.search.as_ref().ok_or_else(|| {
                ToolError::not_supported(
                    "search is not enabled; set search.enabled: true in config",
                )
            })?;

            if matches!(mode.as_str(), "rag" | "both") {
                if self.state.config.search.embedding.endpoint.is_empty() {
                    return Err(ToolError::not_supported(
                        "mode rag/both requires search.embedding.endpoint to be configured",
                    ));
                }
                if !backend.supported_modes().contains(&mode.as_str()) {
                    return Err(ToolError::not_supported(format!(
                        "mode '{mode}' is not supported by the current search backend"
                    )));
                }
            }

            let (results, mode_used) = match mode.as_str() {
                "bm25" => {
                    let r = backend.query_bm25(&a.mount_id, &a.query, top_k).await?;
                    (r, "bm25".to_string())
                }
                "rag" => {
                    let r = backend.query_vector(&a.mount_id, &a.query, top_k).await?;
                    (r, "rag".to_string())
                }
                "both" => {
                    let bm25 =
                        backend.query_bm25(&a.mount_id, &a.query, top_k).await.unwrap_or_default();
                    let vec = backend
                        .query_vector(&a.mount_id, &a.query, top_k)
                        .await
                        .unwrap_or_default();
                    let merged = rrf_merge(&bm25, &vec);
                    (merged, "both".to_string())
                }
                other => {
                    return Err(ToolError::invalid_argument(format!(
                        "unknown mode '{other}', expected bm25, rag, or both"
                    )));
                }
            };

            let warning: Option<&str> = if matches!(mode_used.as_str(), "bm25" | "both") {
                match backend.stats(&a.mount_id).await {
                    Ok(s) if !s.bm25_warm => {
                        Some("BM25 index is not warm; run search.index to populate it")
                    }
                    _ => None,
                }
            } else {
                None
            };

            let result_values: Vec<Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "path": r.path,
                        "score": r.score,
                        "chunk": r.chunk,
                        "rank": r.rank,
                    })
                })
                .collect();

            Ok(serde_json::json!({
                "results": result_values,
                "mode_used": mode_used,
                "warning": warning,
            }))
        }
        .await;
        to_call_result("search.query", out)
    }

    #[tool(name = "search.delete", description = "Remove a path from the search index.")]
    async fn search_delete(
        &self,
        Parameters(a): Parameters<SearchDeleteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.authorize(&a.mount_id, &self.person).await?;
            let path = self.norm(&a.path)?;
            let _recursive = a.recursive;

            let backend = self.state.search.as_ref().ok_or_else(|| {
                ToolError::not_supported(
                    "search is not enabled; set search.enabled: true in config",
                )
            })?;

            let deleted = backend.delete_path(&a.mount_id, &path).await?;

            Ok(serde_json::json!({ "deleted": deleted }))
        }
        .await;
        to_call_result("search.delete", out)
    }

    #[tool(name = "search.status", description = "Report index statistics for this volume.")]
    async fn search_status(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            self.state.authorize(&a.mount_id, &self.person).await?;

            let backend = self.state.search.as_ref().ok_or_else(|| {
                ToolError::not_supported(
                    "search is not enabled; set search.enabled: true in config",
                )
            })?;

            let stats = backend.stats(&a.mount_id).await?;

            Ok(serde_json::json!({
                "bm25_docs": stats.bm25_docs,
                "vector_chunks": stats.vector_chunks,
                "mode": stats.mode,
                "bm25_warm": stats.bm25_warm,
            }))
        }
        .await;
        to_call_result("search.status", out)
    }

    // ── git.* core family (39 tools) ──────────────────────────────────────

    #[tool(name = "git.init", description = "Initialize the volume as a git repository.")]
    async fn git_init(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_init(self.git_ctx(), a, None).await;
        to_call_result("git.init", out)
    }

    #[tool(name = "git.status", description = "Show HEAD, current branch, and all refs.")]
    async fn git_status(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_status(self.git_ctx(), a, None).await;
        to_call_result("git.status", out)
    }

    #[tool(name = "git.branches", description = "List all branches with their SHA.")]
    async fn git_branches(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_branches(self.git_ctx(), a, None).await;
        to_call_result("git.branches", out)
    }

    #[tool(
        name = "git.branch_create",
        description = "Create a branch at a start point, optionally checking it out."
    )]
    async fn git_branch_create(
        &self,
        Parameters(a): Parameters<GitBranchCreateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_branch_create(self.git_ctx(), a, None).await;
        to_call_result("git.branch_create", out)
    }

    #[tool(
        name = "git.branch_switch",
        description = "Switch HEAD to an existing branch, rewriting the volume to match its commit."
    )]
    async fn git_branch_switch(
        &self,
        Parameters(a): Parameters<GitBranchSwitchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_branch_switch(self.git_ctx(), a, None).await;
        to_call_result("git.branch_switch", out)
    }

    #[tool(
        name = "git.branch_delete",
        description = "Delete a branch. Refuses the branch currently checked out, and refuses a branch holding commits reachable from no other ref unless force is true. Removes the ref only: every commit stays in the object store, so a mistaken delete is undone by recreating the branch at the reported sha with git.branch_create."
    )]
    async fn git_branch_delete(
        &self,
        Parameters(a): Parameters<GitBranchDeleteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_branch_delete(self.git_ctx(), a, None).await;
        to_call_result("git.branch_delete", out)
    }

    #[tool(
        name = "git.branch_reset",
        description = "Move a branch pointer to another commit, the equivalent of git branch -f. Refuses a move that is not a fast-forward unless force is true, and always reports old_sha so a mistaken move is undone by moving back to it. On the branch currently checked out it also rewrites the volume to the target commit's tree, discarding uncommitted changes; on any other branch it moves the ref and leaves every file alone."
    )]
    async fn git_branch_reset(
        &self,
        Parameters(a): Parameters<GitBranchResetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_branch_reset(self.git_ctx(), a, None).await;
        to_call_result("git.branch_reset", out)
    }

    #[tool(
        name = "git.reset",
        description = "Move the current branch's pointer to another commit, the equivalent of git reset. Mode soft moves the pointer alone and leaves every volume file exactly as it is, so the changes of the commits left behind stay in the volume ready to be committed again. Mode hard also rewrites the volume to the target commit's tree, discarding uncommitted changes. There is no mixed mode. Commits the branch no longer reaches are orphaned, never deleted, so a mistaken reset is undone by resetting back to the reported old_sha."
    )]
    async fn git_reset(
        &self,
        Parameters(a): Parameters<GitResetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_reset(self.git_ctx(), a, None).await;
        to_call_result("git.reset", out)
    }

    #[tool(name = "git.tags", description = "List all tags.")]
    async fn git_tags(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_tags(self.git_ctx(), a, None).await;
        to_call_result("git.tags", out)
    }

    #[tool(name = "git.log", description = "List commits. ref_name defaults to HEAD.")]
    async fn git_log(
        &self,
        Parameters(a): Parameters<GitLogArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_log(self.git_ctx(), a, None).await;
        to_call_result("git.log", out)
    }

    #[tool(name = "git.show", description = "Show details and diff of a commit.")]
    async fn git_show(
        &self,
        Parameters(a): Parameters<GitShowArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_show(self.git_ctx(), a, None).await;
        to_call_result("git.show", out)
    }

    #[tool(
        name = "git.diff",
        description = "Show diff between two refs or a ref and working tree."
    )]
    async fn git_diff(
        &self,
        Parameters(a): Parameters<GitDiffArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_diff(self.git_ctx(), a, None).await;
        to_call_result("git.diff", out)
    }

    #[tool(
        name = "git.commit",
        description = "Create a commit from the current state of the volume."
    )]
    async fn git_commit(
        &self,
        Parameters(a): Parameters<GitCommitArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_commit(self.git_ctx(), a, None).await;
        to_call_result("git.commit", out)
    }

    #[tool(
        name = "git.checkout_file",
        description = "Restore a file from a commit into the volume."
    )]
    async fn git_checkout_file(
        &self,
        Parameters(a): Parameters<GitCheckoutFileArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_checkout_file(self.git_ctx(), a, None).await;
        to_call_result("git.checkout_file", out)
    }

    #[tool(name = "git.blame", description = "Show who last modified each line of a file.")]
    async fn git_blame(
        &self,
        Parameters(a): Parameters<GitBlameArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_blame(self.git_ctx(), a, None).await;
        to_call_result("git.blame", out)
    }

    #[tool(
        name = "git.remote_add",
        description = "Record a named remote for the volume, so more than one repository can be pushed to, fetched from or pulled from. The URL must be https, must not embed credentials, and its host must be declared in git.hosts; store the credential with git.auth instead. A name already in use is refused rather than overwritten."
    )]
    async fn git_remote_add(
        &self,
        Parameters(a): Parameters<GitRemoteAddArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_remote_add(self.git_ctx(), a, None).await;
        to_call_result("git.remote_add", out)
    }

    #[tool(
        name = "git.remote_remove",
        description = "Delete a named remote and every remote-tracking ref under refs/remotes/{name}/. Branches, commits and files are untouched, and the objects fetched from that remote are kept. A name matching no remote is an error, never a silent no-op."
    )]
    async fn git_remote_remove(
        &self,
        Parameters(a): Parameters<GitRemoteRemoveArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_remote_remove(self.git_ctx(), a, None).await;
        to_call_result("git.remote_remove", out)
    }

    #[tool(
        name = "git.remote_list",
        description = "List every remote recorded for the volume with its resolved host and provider. A volume with no remote returns an empty list. No credential is ever included."
    )]
    async fn git_remote_list(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_remote_list(self.git_ctx(), a, None).await;
        to_call_result("git.remote_list", out)
    }

    #[tool(
        name = "git.remote_clone",
        description = "Clone a remote git repository (GitHub, GitLab, or any HTTPS URL) into a volume. Uses the OAuth token stored by git.auth "
    )]
    async fn git_remote_clone(
        &self,
        Parameters(a): Parameters<GitRemoteCloneArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_remote_clone(self.git_ctx(), a, None, None).await;
        to_call_result("git.remote_clone", out)
    }

    #[tool(
        name = "git.remote_fetch",
        description = "Fetch objects and update refs/remotes/{remote}/* from a declared remote. Never advances a local branch and never touches a working tree file. A branch removed on the remote is reported in refs_stale, not pruned locally."
    )]
    async fn git_remote_fetch(
        &self,
        Parameters(a): Parameters<GitRemoteFetchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_remote_fetch(self.git_ctx(), a, None, None).await;
        to_call_result("git.remote_fetch", out)
    }

    #[tool(
        name = "git.remote_pull",
        description = "Fetch from a declared remote, then advance the checked-out branch to the remote tip and update the volume's files to match. A fast-forward applies directly. A diverged history is merged: a clean three-way merge creates a merge commit and updates the volume, a conflicting one applies nothing and returns status conflict with both sides' content of every conflicting file, to be finished with git.merge_resolve or abandoned with git.merge_abort. Refuses a dirty volume (commit or discard first) and refuses any branch other than the one currently checked out."
    )]
    async fn git_remote_pull(
        &self,
        Parameters(a): Parameters<GitRemotePullArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_remote_pull(self.git_ctx(), a, None, None).await;
        to_call_result("git.remote_pull", out)
    }

    #[tool(
        name = "git.remote_push",
        description = "Push a local branch to a declared remote, under the same name unless remote_branch names another one. Creates the branch on the remote when it is absent there. Fails if the push is not a fast-forward, unless force is true, which requires expected_remote_sha to state the remote sha being overwritten."
    )]
    async fn git_remote_push(
        &self,
        Parameters(a): Parameters<GitRemotePushArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_remote_push(self.git_ctx(), a, None, None).await;
        to_call_result("git.remote_push", out)
    }

    #[tool(
        name = "git.merge",
        description = "Merge a branch, tag or commit into the checked-out branch. An already merged source is reported as already_up_to_date and changes nothing. A source the current branch is an ancestor of fast-forwards. Anything else is a three-way merge: a clean one creates a merge commit and updates the volume, a conflicting one applies nothing and returns status conflict with both sides' content of every conflicting file, to be finished with git.merge_resolve or abandoned with git.merge_abort. Refuses a dirty volume (commit or stash first)."
    )]
    async fn git_merge(
        &self,
        Parameters(a): Parameters<GitMergeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_merge(self.git_ctx(), a, None).await;
        to_call_result("git.merge", out)
    }

    #[tool(
        name = "git.merge_abort",
        description = "Abandon the merge left in conflict by git.merge or git.remote_pull, discarding every resolution recorded so far and leaving HEAD and every volume file exactly as they were before the merge started."
    )]
    async fn git_merge_abort(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_merge_abort(self.git_ctx(), a, None).await;
        to_call_result("git.merge_abort", out)
    }

    #[tool(
        name = "git.merge_resolve",
        description = "Finish a merge left in conflict by git.merge or git.remote_pull. Each resolution names one conflicting path and carries exactly one of strategy ('ours' for the checked-out branch's side, 'theirs' for the side being merged in, both taken whole) or content (the exact bytes to use, which is how a caller merges the two sides itself). Paths may be resolved over several calls: until the last one is resolved the merge stays in progress and nothing is written, and once it is the merge commit is created and every resulting file written to the volume as one all-or-nothing unit. A path that is not in conflict rejects the whole call."
    )]
    async fn git_merge_resolve(
        &self,
        Parameters(a): Parameters<GitMergeResolveArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_merge_resolve(self.git_ctx(), a, None).await;
        to_call_result("git.merge_resolve", out)
    }

    #[tool(
        name = "git.cherry_pick",
        description = "Apply the change one commit made onto the checked-out branch, as a NEW commit with a new sha that keeps the original author and records you as the committer. A commit already in the branch's history, or whose change the branch already carries, is reported as already_present and never duplicated. A conflicting pick applies nothing and returns status conflict with both sides' content of every conflicting file, to be finished with git.cherry_pick_continue or abandoned with git.cherry_pick_abort. A merge commit needs mainline, the parent its change is taken relative to. Refuses a dirty volume (commit or stash first)."
    )]
    async fn git_cherry_pick(
        &self,
        Parameters(a): Parameters<GitCherryPickArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let mainline_val = serde_json::to_value(a.mainline).expect("serialize");
            let mainline =
                crate::tools::git::parse_mainline("git.cherry_pick", Some(&mainline_val))?;
            crate::tools::git::tool_cherry_pick(self.git_ctx(), a, mainline, None).await
        }
        .await;
        to_call_result("git.cherry_pick", out)
    }

    #[tool(
        name = "git.cherry_pick_abort",
        description = "Abandon the cherry-pick paused by git.cherry_pick, discarding every resolution recorded, and leaving the branch and every volume file exactly as they were before the pick started."
    )]
    async fn git_cherry_pick_abort(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_cherry_pick_abort(self.git_ctx(), a, None).await;
        to_call_result("git.cherry_pick_abort", out)
    }

    #[tool(
        name = "git.cherry_pick_continue",
        description = "Resume the cherry-pick paused by git.cherry_pick on a conflicting file. Each resolution names one conflicting path and carries exactly one of strategy ('ours' for the branch the commit is being applied onto, 'theirs' for the commit being picked, both taken whole) or content (the exact bytes to use). Paths may be resolved over several calls: until the last one is resolved the pick stays paused and nothing is written. Once every path is resolved the commit is created. A path that is not in conflict rejects the whole call, and so does a call made while something other than a cherry-pick is in progress."
    )]
    async fn git_cherry_pick_continue(
        &self,
        Parameters(a): Parameters<GitResolutionsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_cherry_pick_continue(self.git_ctx(), a, None).await;
        to_call_result("git.cherry_pick_continue", out)
    }

    #[tool(
        name = "git.revert",
        description = "Undo what one commit did by adding a NEW commit on the checked-out branch whose change is the exact inverse, leaving the original commit in history untouched. The default message is Revert followed by the original subject in quotes. Reverting a revert restores the change. Reverting the very first commit of a history removes everything it added. A conflicting revert applies nothing and returns status conflict with both sides' content of every conflicting file, to be finished with git.revert_continue or abandoned with git.revert_abort. A merge commit needs mainline, the parent the revert is computed against. Refuses a dirty volume (commit or stash first)."
    )]
    async fn git_revert(
        &self,
        Parameters(a): Parameters<GitRevertArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = async {
            let mainline_val = serde_json::to_value(a.mainline).expect("serialize");
            let mainline = crate::tools::git::parse_mainline("git.revert", Some(&mainline_val))?;
            crate::tools::git::tool_revert(self.git_ctx(), a, mainline, None).await
        }
        .await;
        to_call_result("git.revert", out)
    }

    #[tool(
        name = "git.revert_abort",
        description = "Abandon the revert paused by git.revert, discarding every resolution recorded, and leaving the branch and every volume file exactly as they were before the revert started."
    )]
    async fn git_revert_abort(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_revert_abort(self.git_ctx(), a, None).await;
        to_call_result("git.revert_abort", out)
    }

    #[tool(
        name = "git.revert_continue",
        description = "Resume the revert paused by git.revert on a conflicting file. Each resolution names one conflicting path and carries exactly one of strategy ('ours' for the branch the revert is being made on, 'theirs' for the state the reverted commit is being undone back to, both taken whole) or content (the exact bytes to use). Paths may be resolved over several calls: until the last one is resolved the revert stays paused and nothing is written. Once every path is resolved the inverse commit is created. A path that is not in conflict rejects the whole call, and so does a call made while something other than a revert is in progress."
    )]
    async fn git_revert_continue(
        &self,
        Parameters(a): Parameters<GitResolutionsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_revert_continue(self.git_ctx(), a, None).await;
        to_call_result("git.revert_continue", out)
    }

    #[tool(
        name = "git.rebase",
        description = "Replay the checked-out branch's commits onto another commit, following an explicit todo list. Each entry names one commit of the range between onto and the branch tip and one action: pick replays it, drop leaves it out, reword replays it with a new message, squash folds it into the entry above it. The whole todo is checked before anything is replayed: an unknown sha, a commit outside the range, a commit of the range left out, a duplicate sha, an unknown action, a leading squash or a blank reword message rejects the call and changes nothing. A branch that already contains onto is reported as up_to_date. Refuses a dirty volume (commit or stash first)."
    )]
    async fn git_rebase(
        &self,
        Parameters(a): Parameters<GitRebaseArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_rebase(self.git_ctx(), a, None).await;
        to_call_result("git.rebase", out)
    }

    #[tool(
        name = "git.rebase_abort",
        description = "Abandon the rebase paused by git.rebase, discarding every commit replayed so far and every resolution recorded, and leaving the branch and every volume file exactly as they were before the rebase started."
    )]
    async fn git_rebase_abort(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_rebase_abort(self.git_ctx(), a, None).await;
        to_call_result("git.rebase_abort", out)
    }

    #[tool(
        name = "git.rebase_continue",
        description = "Resume the rebase paused by git.rebase on a conflicting commit. Each resolution names one conflicting path of the paused commit and carries exactly one of strategy ('ours' for the side already replayed onto, 'theirs' for the commit being replayed, both taken whole) or content (the exact bytes to use). Paths may be resolved over several calls: until the last one is resolved the rebase stays paused at the same commit and nothing is written. Once every path is resolved that commit is replayed and the remaining todo entries follow, until the list is exhausted or a further conflict pauses it again. A path that is not in conflict rejects the whole call, and so does a call made while something other than a rebase is in progress."
    )]
    async fn git_rebase_continue(
        &self,
        Parameters(a): Parameters<GitResolutionsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_rebase_continue(self.git_ctx(), a, None).await;
        to_call_result("git.rebase_continue", out)
    }

    #[tool(
        name = "git.stash_save",
        description = "Set the volume's uncommitted changes aside as a stash entry and rewrite the volume back to HEAD's tree. The entry is a commit holding the exact state the volume was in, identified by an opaque stash_id, and it survives deletion of the branch it was taken from. Refuses a volume that has no uncommitted changes, and refuses once the volume holds git.max_stash_entries entries."
    )]
    async fn git_stash_save(
        &self,
        Parameters(a): Parameters<GitStashSaveArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_stash_save(self.git_ctx(), a, None).await;
        to_call_result("git.stash_save", out)
    }

    #[tool(
        name = "git.stash_list",
        description = "List every stash entry of the volume, most recent first. The pool is per volume, not per branch: an entry taken on one branch is listed whatever branch is checked out, and base_sha names the commit it was taken against."
    )]
    async fn git_stash_list(
        &self,
        Parameters(a): Parameters<MountOnlyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_stash_list(self.git_ctx(), a, None).await;
        to_call_result("git.stash_list", out)
    }

    #[tool(
        name = "git.stash_apply",
        description = "Apply one stash entry's changes onto the current volume state, keeping the entry listed so it can be applied again. The entry applies onto whatever branch is checked out now, even one that did not exist when it was taken. A conflicting application writes nothing to the volume and returns status conflict with both sides' content of every conflicting file, to be finished with git.merge_resolve or abandoned with git.merge_abort."
    )]
    async fn git_stash_apply(
        &self,
        Parameters(a): Parameters<GitStashIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_stash_apply_or_pop(
            self.git_ctx(),
            a,
            None,
            "git.stash_apply",
            false,
        )
        .await;
        to_call_result("git.stash_apply", out)
    }

    #[tool(
        name = "git.stash_pop",
        description = "Apply one stash entry's changes onto the current volume state and delete the entry, but only once the application has fully succeeded: an application that ends in conflict reports status conflict with dropped false and keeps the entry, so no work is lost. The entry applies onto whatever branch is checked out now, even one that did not exist when it was taken. A conflicting application writes nothing to the volume and is finished with git.merge_resolve or abandoned with git.merge_abort."
    )]
    async fn git_stash_pop(
        &self,
        Parameters(a): Parameters<GitStashIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_stash_apply_or_pop(
            self.git_ctx(),
            a,
            None,
            "git.stash_pop",
            true,
        )
        .await;
        to_call_result("git.stash_pop", out)
    }

    #[tool(
        name = "git.stash_drop",
        description = "Delete one stash entry by id, leaving every volume file untouched and every other entry in place. Removes the ref only: the commit stays in the object store."
    )]
    async fn git_stash_drop(
        &self,
        Parameters(a): Parameters<GitStashIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git::tool_stash_drop(self.git_ctx(), a, None).await;
        to_call_result("git.stash_drop", out)
    }

    // ── git.auth* family (4 tools) ────────────────────────────────────

    #[tool(
        name = "git.auth",
        description = "Start OAuth device flow for GitHub or GitLab. Returns user_code and verification_uri."
    )]
    async fn git_auth(
        &self,
        Parameters(a): Parameters<GitAuthArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_auth::tool_auth(self.git_ctx(), a, None, None).await;
        to_call_result("git.auth", out)
    }

    #[tool(name = "git.auth_revoke", description = "Revoke the stored token for a provider.")]
    async fn git_auth_revoke(
        &self,
        Parameters(a): Parameters<GitAuthRevokeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_auth::tool_auth_revoke(self.git_ctx(), a, None).await;
        to_call_result("git.auth_revoke", out)
    }

    #[tool(
        name = "git.auth_status",
        description = "Check authentication status for a provider (or all providers)."
    )]
    async fn git_auth_status(
        &self,
        Parameters(a): Parameters<GitAuthStatusArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_auth::tool_auth_status(self.git_ctx(), a, None).await;
        to_call_result("git.auth_status", out)
    }

    #[tool(
        name = "git.token_set",
        description = "Seed a personal access token you already hold for a host declared in git.hosts, without the interactive device flow. The token is never echoed back."
    )]
    async fn git_token_set(
        &self,
        Parameters(a): Parameters<GitTokenSetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_auth::tool_token_set(self.git_ctx(), a, None).await;
        to_call_result("git.token_set", out)
    }

    // ── git.pr_* family (6 tools) ────────────────────────────────────

    #[tool(
        name = "git.pr_create",
        description = "Open a pull request on GitHub, or a merge request on GitLab, for a declared remote. Fails before contacting the provider when the head branch is not on the remote yet."
    )]
    async fn git_pr_create(
        &self,
        Parameters(a): Parameters<GitPrCreateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_pr::tool_pr_create(self.git_ctx(), a, None, None, None).await;
        to_call_result("git.pr_create", out)
    }

    #[tool(
        name = "git.pr_diff",
        description = "Read the unified diff of one pull request on GitHub, or one merge request on GitLab. The answer is bounded by git.max_pr_diff_mb; a larger diff is returned truncated, with truncated set to true."
    )]
    async fn git_pr_diff(
        &self,
        Parameters(a): Parameters<GitPrDiffArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_pr::tool_pr_diff(self.git_ctx(), a, None, None, None).await;
        to_call_result("git.pr_diff", out)
    }

    #[tool(
        name = "git.pr_get",
        description = "Read one pull request on GitHub, or one merge request on GitLab, normalized to one shape and enriched with its review state and its check state. Fails rather than reporting an unknown review or check state as none."
    )]
    async fn git_pr_get(
        &self,
        Parameters(a): Parameters<GitPrGetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_pr::tool_pr_get(self.git_ctx(), a, None, None, None).await;
        to_call_result("git.pr_get", out)
    }

    #[tool(
        name = "git.pr_list",
        description = "List the pull requests of a declared remote on GitHub, or its merge requests on GitLab, filtered by state and normalized to one shape."
    )]
    async fn git_pr_list(
        &self,
        Parameters(a): Parameters<GitPrListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_pr::tool_pr_list(self.git_ctx(), a, None, None, None).await;
        to_call_result("git.pr_list", out)
    }

    #[tool(
        name = "git.pr_merge",
        description = "Merge a pull request on GitHub, or a merge request on GitLab, with the chosen strategy. The merge happens on the provider: no local ref and no remote-tracking ref is updated, so git.remote_fetch is required to observe it locally. A refusal by the provider (failing checks, missing reviews, a protected branch, a strategy disabled for that repository, an already merged or closed pull request) is reported as the provider's own status and message, never as a success."
    )]
    async fn git_pr_merge(
        &self,
        Parameters(a): Parameters<GitPrMergeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_pr::tool_pr_merge(self.git_ctx(), a, None, None, None).await;
        to_call_result("git.pr_merge", out)
    }

    #[tool(
        name = "git.pr_review",
        description = "Submit a review verdict on a pull request on GitHub, or on a merge request on GitLab: approve it, request changes, or comment. On GitLab, which has no review verdict, approve uses the approve endpoint while the other two leave a note. A refusal by the provider (reviewing one's own pull request, a token the provider no longer accepts) is reported as the provider's own status and message, never as a success."
    )]
    async fn git_pr_review(
        &self,
        Parameters(a): Parameters<GitPrReviewArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let out = crate::tools::git_pr::tool_pr_review(self.git_ctx(), a, None, None, None).await;
        to_call_result("git.pr_review", out)
    }
}

/// Test-only (US-0008, DT-004): one extra tool, `t.notifies_then_returns`,
/// that emits a progress notification before returning, so the transport's
/// SSE-on-notification path has something real to exercise. The whole
/// `impl` block (including the `#[tool_router]` expansion) is behind
/// `#[cfg(test)]`: in a shipped build this item does not exist, not merely
/// an unreachable branch.
#[cfg(test)]
#[tool_router(router = test_tool_router)]
impl McpServer {
    #[tool(
        name = "t.notifies_then_returns",
        description = "Test-only tool: emits a progress notification before returning."
    )]
    async fn t_notifies_then_returns(
        &self,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let notification = rmcp::model::ServerNotification::ProgressNotification(
            rmcp::model::ProgressNotification::new(rmcp::model::ProgressNotificationParam::new(
                rmcp::model::ProgressToken(rmcp::model::NumberOrString::Number(0)),
                1.0,
            )),
        );
        let _ = context.peer.send_notification(notification).await;
        Ok(CallToolResult::success(vec![ContentBlock::text("done")]))
    }
}

/// Wiring required to serve [`McpServer`] over `rmcp`'s streamable HTTP
/// transport (US-0008): the dispatch methods above are already complete
/// (US-0007), but nothing yet told `rmcp` how to answer `initialize` or how
/// to route a call through [`McpServer::tool_router`]. `#[tool_handler]`
/// generates `call_tool`/`list_tools` from that router; only `get_info` is
/// hand-written, to advertise the same server identity the old hand-rolled
/// transport did.
#[rmcp::tool_handler(router = self.tool_router)]
impl rmcp::ServerHandler for McpServer {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        rmcp::model::ServerConfig::new(
            rmcp::model::ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(rmcp::model::Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::admin::test_support::Fixture;

    /// Pulls the JSON payload out of a successful `CallToolResult`, panicking
    /// on an error result (callers assert the happy path, or inspect the raw
    /// result themselves for the conflict/error cases).
    fn ok_json(r: Result<CallToolResult, ErrorData>) -> Value {
        let r = r.expect("ErrorData (protocol error), not a tool error");
        assert_ne!(r.is_error, Some(true), "tool returned an error result: {r:?}");
        let text = r
            .content
            .iter()
            .find_map(|c| match c {
                ContentBlock::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .expect("a text content block");
        serde_json::from_str(&text).expect("tool result must be JSON")
    }

    const NAMES: &[(&str, &str)] = &[
        ("fs.read", "Read a text file with line-numbered, paged output."),
        ("fs.read_bytes", "Read raw bytes (base64) with MIME type."),
        ("fs.glob", "Find files by glob pattern, newest first (cap 100)."),
        ("fs.grep", "Search file contents (files|content|count modes)."),
        ("fs.documentize", "Generate the Markdown companion of a stored document."),
        (
            "admin.create_project",
            "Create a project for a designated owner and provision its volume (platform admin only).",
        ),
        ("admin.list_projects", "List projects the caller can access."),
        (
            "admin.get_index_mode",
            "Get the current search index mode for a project (member or platform admin).",
        ),
        ("search.index", "Index a file or directory into the search engine for this volume."),
        ("search.status", "Report index statistics for this volume."),
        ("git.init", "Initialize the volume as a git repository."),
        ("git.status", "Show HEAD, current branch, and all refs."),
        ("git.log", "List commits. ref_name defaults to HEAD."),
        ("git.show", "Show details and diff of a commit."),
        ("git.tags", "List all tags."),
        (
            "git.auth",
            "Start OAuth device flow for GitHub or GitLab. Returns user_code and verification_uri.",
        ),
        ("git.auth_revoke", "Revoke the stored token for a provider."),
        ("git.auth_status", "Check authentication status for a provider (or all providers)."),
        (
            "git.token_set",
            "Seed a personal access token you already hold for a host declared in git.hosts, without the interactive device flow. The token is never echoed back.",
        ),
        (
            "git.pr_create",
            "Open a pull request on GitHub, or a merge request on GitLab, for a declared remote. Fails before contacting the provider when the head branch is not on the remote yet.",
        ),
        (
            "git.pr_diff",
            "Read the unified diff of one pull request on GitHub, or one merge request on GitLab. The answer is bounded by git.max_pr_diff_mb; a larger diff is returned truncated, with truncated set to true.",
        ),
        (
            "git.pr_get",
            "Read one pull request on GitHub, or one merge request on GitLab, normalized to one shape and enriched with its review state and its check state. Fails rather than reporting an unknown review or check state as none.",
        ),
        (
            "git.pr_list",
            "List the pull requests of a declared remote on GitHub, or its merge requests on GitLab, filtered by state and normalized to one shape.",
        ),
        (
            "git.pr_merge",
            "Merge a pull request on GitHub, or a merge request on GitLab, with the chosen strategy. The merge happens on the provider: no local ref and no remote-tracking ref is updated, so git.remote_fetch is required to observe it locally. A refusal by the provider (failing checks, missing reviews, a protected branch, a strategy disabled for that repository, an already merged or closed pull request) is reported as the provider's own status and message, never as a success.",
        ),
        (
            "git.pr_review",
            "Submit a review verdict on a pull request on GitHub, or on a merge request on GitLab: approve it, request changes, or comment. On GitLab, which has no review verdict, approve uses the approve endpoint while the other two leave a note. A refusal by the provider (reviewing one's own pull request, a token the provider no longer accepts) is reported as the provider's own status and message, never as a success.",
        ),
    ];

    #[test]
    fn fs_tool_count_is_thirty_five() {
        let router = McpServer::tool_router();
        let names: Vec<String> = router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .filter(|n| n.starts_with("fs."))
            .collect();
        assert_eq!(names.len(), 35, "got: {names:?}");
    }

    #[test]
    fn total_tool_count_is_ninety_eight() {
        let router = McpServer::tool_router();
        let names: Vec<String> =
            router.list_all().into_iter().map(|t| t.name.to_string()).collect();
        let fs_count = names.iter().filter(|n| n.starts_with("fs.")).count();
        let admin_count = names.iter().filter(|n| n.starts_with("admin.")).count();
        let search_count = names.iter().filter(|n| n.starts_with("search.")).count();
        let git_count = names.iter().filter(|n| n.starts_with("git.")).count();
        assert_eq!(fs_count, 35, "got: {names:?}");
        assert_eq!(admin_count, 10, "got: {names:?}");
        assert_eq!(search_count, 4, "got: {names:?}");
        assert_eq!(git_count, 49, "got: {names:?}");
        assert_eq!(names.len(), 98, "got: {names:?}");
    }

    #[test]
    fn every_git_auth_and_pr_tool_is_registered() {
        const ALL_GIT_AUTH_PR_TOOLS: [&str; 10] = [
            "git.auth",
            "git.auth_revoke",
            "git.auth_status",
            "git.token_set",
            "git.pr_create",
            "git.pr_diff",
            "git.pr_get",
            "git.pr_list",
            "git.pr_merge",
            "git.pr_review",
        ];
        let router = McpServer::tool_router();
        let names: std::collections::HashSet<String> =
            router.list_all().into_iter().map(|t| t.name.to_string()).collect();
        for name in ALL_GIT_AUTH_PR_TOOLS {
            assert!(names.contains(name), "{name} is missing from the router");
        }
    }

    #[test]
    fn every_core_git_tool_is_registered() {
        const ALL_GIT_TOOLS: [&str; 39] = [
            "git.init",
            "git.status",
            "git.branches",
            "git.branch_create",
            "git.branch_switch",
            "git.branch_delete",
            "git.branch_reset",
            "git.reset",
            "git.tags",
            "git.log",
            "git.show",
            "git.diff",
            "git.commit",
            "git.checkout_file",
            "git.blame",
            "git.merge",
            "git.merge_resolve",
            "git.merge_abort",
            "git.rebase",
            "git.rebase_continue",
            "git.rebase_abort",
            "git.cherry_pick",
            "git.cherry_pick_continue",
            "git.cherry_pick_abort",
            "git.revert",
            "git.revert_continue",
            "git.revert_abort",
            "git.stash_save",
            "git.stash_list",
            "git.stash_drop",
            "git.stash_apply",
            "git.stash_pop",
            "git.remote_add",
            "git.remote_remove",
            "git.remote_list",
            "git.remote_clone",
            "git.remote_push",
            "git.remote_fetch",
            "git.remote_pull",
        ];
        let router = McpServer::tool_router();
        let names: std::collections::HashSet<String> =
            router.list_all().into_iter().map(|t| t.name.to_string()).collect();
        for name in ALL_GIT_TOOLS {
            assert!(names.contains(name), "{name} is missing from the router");
        }
    }

    const OWNER: &str = "owner@test.com";
    const MOUNT: &str = "gitproj";
    const CFG: &str = "/src/config.toml";
    const BASE_CFG: &str =
        "[server]\nport = 8080\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n";
    const MAIN_CFG: &str =
        "[server]\nport = 8000\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n";
    const FEATURE_CFG_CONFLICT: &str =
        "[server]\nport = 9090\nhost = \"0.0.0.0\"\nworkers = 4\ntimeout = 30\nretries = 2\n";

    async fn git_env() -> (Fixture, McpServer) {
        let f = Fixture::with_config(|c| c.git.enabled = true).await;
        f.seed_project(MOUNT, OWNER).await;
        let server = McpServer::new(f.state.clone(), OWNER.to_string());
        (f, server)
    }

    async fn write(f: &Fixture, path: &str, content: &str) {
        let client = f.state.stores.client(MOUNT).await.unwrap();
        client.write_text_atomic(path, content).await.unwrap();
    }

    /// E2E-GIT-002: a genuine merge conflict (mirrors `tools::git`'s own
    /// `seed_conflict`/`e2e_new_502_conflict_response_shape` fixture) produces
    /// the exact same `status: "conflict"` shape through the new `#[tool]`
    /// method, because it runs through the exact same handler.
    #[tokio::test]
    async fn git_merge_tool_reports_conflict_shape_like_the_old_handler() {
        let (f, server) = git_env().await;

        ok_json(server.git_init(Parameters(MountOnlyArgs { mount_id: MOUNT.to_string() })).await);
        write(&f, CFG, BASE_CFG).await;
        let c0 = ok_json(
            server
                .git_commit(Parameters(GitCommitArgs {
                    mount_id: MOUNT.to_string(),
                    message: "C0".to_string(),
                    author_name: None,
                    author_email: None,
                }))
                .await,
        )["commit_sha"]
            .as_str()
            .unwrap()
            .to_string();

        ok_json(
            server
                .git_branch_create(Parameters(GitBranchCreateArgs {
                    mount_id: MOUNT.to_string(),
                    name: "feature".to_string(),
                    start_point: c0,
                    checkout: true,
                }))
                .await,
        );
        write(&f, CFG, FEATURE_CFG_CONFLICT).await;
        ok_json(
            server
                .git_commit(Parameters(GitCommitArgs {
                    mount_id: MOUNT.to_string(),
                    message: "C1".to_string(),
                    author_name: None,
                    author_email: None,
                }))
                .await,
        );

        ok_json(
            server
                .git_branch_switch(Parameters(GitBranchSwitchArgs {
                    mount_id: MOUNT.to_string(),
                    name: "main".to_string(),
                }))
                .await,
        );
        write(&f, CFG, MAIN_CFG).await;
        ok_json(
            server
                .git_commit(Parameters(GitCommitArgs {
                    mount_id: MOUNT.to_string(),
                    message: "C2".to_string(),
                    author_name: None,
                    author_email: None,
                }))
                .await,
        );

        let out = ok_json(
            server
                .git_merge(Parameters(GitMergeArgs {
                    mount_id: MOUNT.to_string(),
                    source_ref: "feature".to_string(),
                    squash: false,
                    message: None,
                }))
                .await,
        );

        // Byte-for-byte the same conflict shape `tools::git`'s own
        // `e2e_new_502_conflict_response_shape` asserts on the old handler.
        assert_eq!(out["status"], "conflict");
        assert_eq!(out["operation"], "merge");
        assert_eq!(out["source_ref"], "feature");
        assert_eq!(out["current_step"], Value::Null);
        assert_eq!(out["total_steps"], Value::Null);
        assert_eq!(out["continue_with"], "git.merge_resolve");
        assert_eq!(out["abort_with"], "git.merge_abort");
        let conflicts = out["conflicts"].as_array().expect("conflicts array");
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0]["path"], CFG);
    }

    /// `tracing_subscriber`'s `MakeWriter` over a shared buffer, same pattern
    /// as `tools::git_pr::tests::no_token_value_reaches_a_tracing_span_or_a_log_line`.
    struct CaptureWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CaptureWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// E2E-GITAUTH-002: a fake token value passed to the new `git.token_set`
    /// `#[tool]` method never reaches a captured tracing span or log line,
    /// whether the call succeeds or fails.
    #[tokio::test]
    async fn git_token_set_tool_never_logs_the_token_value() {
        const FAKE_TOKEN: &str = "FAKE_SECRET_TOKEN_zzz_never_logged_123";

        let (_f, server) = git_env().await;

        let buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || CaptureWriter(writer.clone()))
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);

        let _ = server
            .git_token_set(Parameters(GitTokenSetArgs {
                host: "example-not-declared.test".to_string(),
                token: FAKE_TOKEN.to_string(),
                expires_at: None,
            }))
            .await;

        drop(guard);
        let captured = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
        assert!(!captured.contains(FAKE_TOKEN), "token leaked into logs: {captured}");
    }

    fn normalize_schema(v: &serde_json::Value, defs: &serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::Object(map) => {
                let mut out = serde_json::Map::new();
                if let Some(Value::String(ref_path)) = map.get("$ref") {
                    let key = ref_path.rsplit('/').next().unwrap_or(ref_path);
                    if let Some(target) = defs.get(key)
                        && let Value::Object(resolved) = normalize_schema(target, defs)
                    {
                        out = resolved;
                    }
                }
                for (k, val) in map {
                    if k == "description"
                        || k == "$schema"
                        || k == "$defs"
                        || k == "format"
                        || k == "$ref"
                    {
                        continue;
                    }
                    if k == "required"
                        && let Some(arr) = val.as_array()
                    {
                        let mut sorted: Vec<String> = arr
                            .iter()
                            .map(|x| x.as_str().unwrap_or_default().to_string())
                            .collect();
                        sorted.sort();
                        out.insert(
                            k.clone(),
                            Value::Array(sorted.into_iter().map(Value::String).collect()),
                        );
                        continue;
                    }
                    if k == "type" {
                        // Collapse a nullable union (e.g. ["string","null"]) to the
                        // single non-null type: golden models optionality via
                        // `default: null` on a plain type, not a type union.
                        if let Some(arr) = val.as_array() {
                            let non_null: Vec<&Value> =
                                arr.iter().filter(|t| t.as_str() != Some("null")).collect();
                            if non_null.len() == 1 {
                                out.insert(k.clone(), non_null[0].clone());
                                continue;
                            }
                        }
                    }
                    out.insert(k.clone(), normalize_schema(val, defs));
                }
                Value::Object(out)
            }
            Value::Array(arr) => {
                Value::Array(arr.iter().map(|x| normalize_schema(x, defs)).collect())
            }
            other => other.clone(),
        }
    }

    /// US-0007/DT-005: every one of the 94 frozen `#[tool]` schemas structurally
    /// matches `tool-contract-golden.json`, modulo representational choices that
    /// carry no client-observable difference: `$ref`/`$defs` indirection vs. an
    /// inlined object (both resolve to the same validated shape), a nullable
    /// type union `["T","null"]` vs. a plain type with `default: null` (both
    /// accept and default the same values), and `required` array order (a set,
    /// not a sequence, in JSON Schema). `description` text and the transport
    /// level `$schema`/`format` annotations are deliberately excluded: DR-005's
    /// byte-identical bar is restated per Invariant 3 as being about the HTTP
    /// response bytes a client observes, not an internal struct's serde field
    /// order, and the frozen contract's own descriptions are exercised verbatim
    /// by `contract_golden.rs` against the OLD `ToolRegistry` path, not this one.
    #[test]
    fn ninety_four_tool_schemas_match_the_golden_contract_structurally() {
        let router = McpServer::tool_router();
        let tools = router.list_all();
        let frozen = crate::tools::contract_golden::frozen_tools()
            .expect("tool-contract-golden.json must be present");

        let mut checked = 0;
        for tool in &tools {
            if tool.name.starts_with("search.") {
                continue; // config-gated, not part of the frozen 94-tool surface
            }
            checked += 1;
            let entry =
                frozen.iter().find(|t| t["name"] == tool.name.as_ref()).unwrap_or_else(|| {
                    panic!("{} is missing from tool-contract-golden.json", tool.name)
                });

            let raw = serde_json::Value::Object(tool.input_schema.as_ref().clone());
            let empty = serde_json::Value::Null;
            let defs = raw.get("$defs").unwrap_or(&empty);
            let mine = normalize_schema(&raw, defs);
            let gold = normalize_schema(&entry["inputSchema"], &empty);
            assert_eq!(mine, gold, "schema structurally drifted on {}", tool.name);
        }
        assert_eq!(checked, 94, "the frozen contract covers exactly 94 non-search tools");
    }

    /// US-0007/DT-007: `tools/list` (the router's own `list_all`) returns exactly
    /// the 94 names `TOOL_CONTRACT.txt` documents, as a set: no tool registered
    /// and undocumented, none documented and missing.
    #[test]
    fn tool_router_lists_exactly_the_94_contract_names() {
        let router = McpServer::tool_router();
        let names: std::collections::BTreeSet<String> = router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .filter(|n| !n.starts_with("search."))
            .collect();

        let frozen = crate::tools::contract_golden::frozen_tools()
            .expect("tool-contract-golden.json must be present");
        let expected: std::collections::BTreeSet<String> =
            frozen.iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();

        assert_eq!(names.len(), 94, "got: {names:?}");
        assert_eq!(expected.len(), 94, "the golden contract itself must hold 94 names");
        let missing: Vec<&String> = expected.difference(&names).collect();
        let extra: Vec<&String> = names.difference(&expected).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "name set drift: missing from router {missing:?}, extra in router {extra:?}"
        );
    }

    #[test]
    fn spot_checked_tool_names_and_descriptions_match_the_contract_verbatim() {
        let router = McpServer::tool_router();
        let tools = router.list_all();
        for (name, description) in NAMES {
            let found = tools.iter().find(|t| t.name == *name).unwrap_or_else(|| {
                panic!("tool '{name}' not found in router");
            });
            assert_eq!(
                found.description.as_deref(),
                Some(*description),
                "description mismatch for {name}"
            );
        }
    }
}
