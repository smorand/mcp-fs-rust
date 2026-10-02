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
use serde::Deserialize;
use serde_json::Value;

use crate::core::fs_ops;
use crate::errors::ToolError;
use crate::search::indexer;
use crate::state::AppState;
use crate::storage::VolumeClient;

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
        Self { state, person, tool_router: Self::tool_router() }
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
    pub exclude_patterns: Vec<String>,
}
fn def_root() -> String {
    "/".to_string()
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
    pub include_glob: Option<String>,
    /// Glob excluding files from the search.
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
    pub exclude_patterns: Vec<String>,
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MountOnlyArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AuditLogArgs {
    /// Project/volume id the operation targets.
    pub mount_id: String,
    /// Only return entries at or after this Unix timestamp (seconds).
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

#[tool_router(router = tool_router)]
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
            fs_ops::glob_files(&client, &root, &a.pattern, &a.exclude_patterns).await
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
            fs_ops::tree(&client, &path, a.max_depth, &a.exclude_patterns, a.with_sizes).await
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
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[(&str, &str)] = &[
        ("fs.read", "Read a text file with line-numbered, paged output."),
        ("fs.read_bytes", "Read raw bytes (base64) with MIME type."),
        ("fs.glob", "Find files by glob pattern, newest first (cap 100)."),
        ("fs.grep", "Search file contents (files|content|count modes)."),
        ("fs.documentize", "Generate the Markdown companion of a stored document."),
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
