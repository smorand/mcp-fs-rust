//! The shared merge engine: one implementation of "combine two histories, and
//! report what could not be combined", plus the single serialized shape every
//! combine operation reports a conflict with.
//!
//! Extracted from the pull path in [`crate::tools::git`] so that `git.merge`,
//! `git.remote_pull` and (later) rebase, cherry_pick, revert and stash apply
//! share one three way merge, one atomic volume apply and one conflict
//! response. Before this module the pull owned all three privately, which is
//! how a second consumer would have grown a second merge with subtly
//! different behaviour.
//!
//! The response shapes below are types, not hand built JSON objects
//! (FR-NEW-186, FR-NEW-199): the specification's audit found field names
//! drifting between requirements and tests four times, and a single serializer
//! turns that class of drift into a compile error instead of a wire change.

use crate::errors::{Result, ToolError};
use crate::git::db::GitOpType;
use crate::safety::SafetyManager;
use crate::storage::VolumeClient;
use git2::{MergeOptions, Oid, Repository, Tree};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// One side of one conflicting path, exactly `exists` and `content`
/// (FR-NEW-186). `content` is null when the side deleted the path and when the
/// path is binary, so bytes are never mangled into text.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConflictSide {
    pub exists: bool,
    pub content: Option<String>,
}

impl ConflictSide {
    /// The side that deleted, or never had, the path (DEC-909: a delete versus
    /// modify is surfaced as a conflict, not refused).
    fn absent() -> Self {
        Self { exists: false, content: None }
    }
}

/// One conflicting path, exactly `path`, `ours`, `theirs`, `base`, `binary`,
/// `type_change` (FR-NEW-186).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConflictFile {
    pub path: String,
    pub ours: ConflictSide,
    pub theirs: ConflictSide,
    pub base: ConflictSide,
    pub binary: bool,
    pub type_change: bool,
}

/// The conflict response, identical for every combine operation, so a caller
/// writes one handler (FR-NEW-170, FR-NEW-186).
///
/// `current_step` and `total_steps` are top level and null for a single step
/// operation; a rebase always reports integers (FR-NEW-187). The forbidden
/// keys (`conflicting_paths`, `resolve_with`, `step`, `target_ref`, `present`,
/// `size`) are absent by construction: this struct is the only way a conflict
/// reaches the wire.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConflictResponse {
    pub status: &'static str,
    pub operation: &'static str,
    pub operation_id: String,
    pub source_ref: Option<String>,
    pub current_step: Option<i64>,
    pub total_steps: Option<i64>,
    pub conflicts: Vec<ConflictFile>,
    pub continue_with: &'static str,
    pub abort_with: &'static str,
}

impl ConflictResponse {
    /// Build the response for `op`, whose completion pair comes from
    /// [`GitOpType`] itself (FR-NEW-241), never from a per call site guess.
    pub fn new(
        op: GitOpType,
        operation_id: String,
        source_ref: Option<String>,
        steps: Option<(i64, i64)>,
        conflicts: Vec<ConflictFile>,
    ) -> Self {
        Self {
            status: "conflict",
            operation: op.as_str(),
            operation_id,
            source_ref,
            current_step: steps.map(|(c, _)| c),
            total_steps: steps.map(|(_, t)| t),
            conflicts,
            continue_with: op.continue_with(),
            abort_with: op.abort_with(),
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// The success response of `git.merge`, exactly the keys FR-NEW-199 lists.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MergeResponse {
    pub status: &'static str,
    pub merge_commit: Option<String>,
    pub fast_forward: bool,
    pub squashed: bool,
    pub files_changed: usize,
}

impl MergeResponse {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// The success or partial response of `git.merge_resolve`, exactly the keys
/// FR-NEW-199 lists. `remaining_conflicts` is ALWAYS an array of path strings,
/// never a count, and it is empty exactly when `status` is `merged`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResolveResponse {
    pub status: &'static str,
    pub merge_commit: Option<String>,
    pub remaining_conflicts: Vec<String>,
    pub resolved_count: usize,
    pub files_changed: usize,
}

impl ResolveResponse {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// The response of `git.rebase`, exactly the keys FR-NEW-210 lists.
///
/// `replayed`, `dropped` and `squashed` are always integers, never null, so a
/// caller reads one shape whether the rebase replayed a hundred commits or
/// nothing at all. `operation_id` is absent, not null, when the rebase finished
/// without pausing, matching the merge family's rule that a key only appears
/// when there is an operation to name.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RebaseResponse {
    pub status: &'static str,
    pub branch: String,
    pub new_tip: String,
    pub replayed: usize,
    pub dropped: usize,
    pub squashed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

impl RebaseResponse {
    /// FR-NEW-217: the branch already contains `onto`, so nothing is replayed
    /// and the tip is the very sha it was, not a rewritten equivalent.
    pub fn up_to_date(branch: String, tip: String) -> Self {
        Self {
            status: "up_to_date",
            branch,
            new_tip: tip,
            replayed: 0,
            dropped: 0,
            squashed: 0,
            operation_id: None,
        }
    }

    /// FR-NEW-210: the whole todo replayed without a single conflicting step,
    /// so there is no operation to name and no pause to continue from. The
    /// status is `completed`, never `ok`: `ok` is emitted by no tool.
    pub fn completed(
        branch: String,
        new_tip: String,
        replayed: usize,
        dropped: usize,
        squashed: usize,
    ) -> Self {
        Self {
            status: "completed",
            branch,
            new_tip,
            replayed,
            dropped,
            squashed,
            operation_id: None,
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// The response of `git.merge_abort` and `git.rebase_abort`, exactly `status`
/// and `operation`, plus `restored_sha` for the operations whose abort names
/// the tip it put back (`git.cherry_pick_abort`). The field is skipped rather
/// than null when the operation does not report one, so the two older tools
/// keep the exact key set they were frozen with.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AbortResponse {
    pub status: &'static str,
    pub operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_sha: Option<String>,
}

impl AbortResponse {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// The response of `git.cherry_pick` and `git.cherry_pick_continue`, exactly
/// `status`, `new_sha` and `source_sha` (FR-NEW-235, FR-NEW-236).
///
/// `status` is `committed` or `already_present`; a conflicting pick reports
/// through [`ConflictResponse`] like every other combine operation. `ok` is
/// emitted by no tool. `new_sha` is null exactly when no commit was created.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CherryPickResponse {
    pub status: &'static str,
    pub new_sha: Option<String>,
    pub source_sha: String,
}

impl CherryPickResponse {
    pub fn committed(new_sha: String, source_sha: String) -> Self {
        Self { status: "committed", new_sha: Some(new_sha), source_sha }
    }

    /// FR-NEW-236: the change the commit carries is already in this branch,
    /// whether by sha (it is an ancestor) or by content (replaying it changes
    /// nothing). Reported, never duplicated, and never an error.
    pub fn already_present(source_sha: String) -> Self {
        Self { status: "already_present", new_sha: None, source_sha }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// The response of `git.revert` and `git.revert_continue`, exactly `status`,
/// `new_sha` and `reverted_sha` (FR-NEW-260, FR-NEW-266).
///
/// `status` is `committed` or `already_present`; a conflicting revert reports
/// through [`ConflictResponse`] like every other combine operation. `ok` is
/// emitted by no tool. `new_sha` is null exactly when no commit was created.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RevertResponse {
    pub status: &'static str,
    pub new_sha: Option<String>,
    pub reverted_sha: String,
}

impl RevertResponse {
    pub fn committed(new_sha: String, reverted_sha: String) -> Self {
        Self { status: "committed", new_sha: Some(new_sha), reverted_sha }
    }

    /// The inverse change is already carried by the branch: replaying it
    /// produces the very tree the branch already has, so there is nothing to
    /// commit. Reported, never duplicated, and never an error.
    pub fn already_present(reverted_sha: String) -> Self {
        Self { status: "already_present", new_sha: None, reverted_sha }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Which side of a conflict a per file strategy names (FR-NEW-174). `ours` is
/// the side the operation is applied onto, `theirs` the side being applied,
/// matching git's own convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Ours,
    Theirs,
}

impl Side {
    /// Exact, lowercase only, mirroring `parse_on_conflict` (FR-NEW-179).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ours" => Some(Self::Ours),
            "theirs" => Some(Self::Theirs),
            _ => None,
        }
    }
}

/// One caller decision about one conflicting path (FR-NEW-174, FR-NEW-175).
///
/// Externally tagged on purpose: the serialized form recorded in the
/// `git_operations.resolutions` column is `{"strategy":"ours"}` or
/// `{"content":"..."}`, which is the same vocabulary the tool argument uses, so
/// a paused operation is readable in the database without a decoder ring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    Strategy(Side),
    Content(String),
}

/// The `operation` object of `git.status`, exactly the keys FR-NEW-286 lists.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OperationSummary {
    pub op_type: &'static str,
    pub source_ref: Option<String>,
    pub current_step: Option<i64>,
    pub total_steps: Option<i64>,
    pub remaining_conflicts: Vec<String>,
    pub continue_with: &'static str,
    pub abort_with: &'static str,
}

/// What a three way merge reached: one tree to apply, or a set of conflicts to
/// report and apply nothing (FR-NEW-171).
pub enum MergeOutcome {
    Clean { tree: Oid },
    Conflicted { conflicts: Vec<ConflictFile> },
}

/// Merge `theirs` into `ours` in memory, with `favor` resolving every
/// conflicting region when it is set (the pull's historical `on_conflict`
/// behaviour) and surfacing conflicts to the caller when it is not (DEC-902).
///
/// Nothing is written: the index lives in memory, the merged tree is written
/// to the object database only on the clean path, and the conflicting path set
/// is read out of the same index. Complexity is libgit2's: O(paths changed on
/// either side) for the merge itself, plus O(bytes of the conflicting files)
/// to read the three sides out. A clean merge reads no file content at all.
pub fn three_way_merge(
    repo: &Repository,
    ours: &git2::Commit<'_>,
    theirs: &git2::Commit<'_>,
    favor: Option<git2::FileFavor>,
) -> Result<MergeOutcome> {
    let mut opts = MergeOptions::new();
    if let Some(f) = favor {
        opts.file_favor(f);
    }
    let mut index = repo
        .merge_commits(ours, theirs, Some(&opts))
        .map_err(|e| ToolError::internal(format!("merge commits: {e}")))?;

    if index.has_conflicts() {
        return Ok(MergeOutcome::Conflicted { conflicts: collect_conflicts(repo, &index)? });
    }
    let tree = index
        .write_tree_to(repo)
        .map_err(|e| ToolError::internal(format!("write merged tree: {e}")))?;
    Ok(MergeOutcome::Clean { tree })
}

/// Rebuild the merged tree of a conflicted merge with every conflicting path
/// forced to its resolved bytes (FR-NEW-174 through FR-NEW-176).
///
/// The merge is replayed rather than cached, because the cache would be the
/// whole conflicted index and this operation survives a server restart
/// (FR-NEW-278): the inputs are two commits and a decision per path, which is
/// exactly what the relational row holds. A strategy takes that side's blob
/// WHOLE, never a per hunk favor, so `ours` really means "this side's file".
///
/// Complexity: libgit2's merge, O(paths changed on either side), plus O(1)
/// index operations per resolved path and O(bytes) only for the content
/// resolutions, whose bytes the caller supplied anyway. Nothing reads the
/// volume and no file content is cloned.
pub fn resolve_merged_tree(
    repo: &Repository,
    ours: &git2::Commit<'_>,
    theirs: &git2::Commit<'_>,
    resolutions: &BTreeMap<String, Resolution>,
) -> Result<Oid> {
    let mut index = repo
        .merge_commits(ours, theirs, None)
        .map_err(|e| ToolError::internal(format!("merge commits: {e}")))?;
    resolve_index(repo, &mut index, resolutions)
}

/// [`resolve_merged_tree`] over an index the caller already produced, which is
/// what a rebase step needs: its index comes from `cherrypick_commit`, whose
/// base is the replayed commit's own parent rather than the merge base two
/// branch tips share. Same resolution vocabulary, same all-or-nothing rule.
pub fn resolve_index(
    repo: &Repository,
    index: &mut git2::Index,
    resolutions: &BTreeMap<String, Resolution>,
) -> Result<Oid> {
    // The stage entries, read out before the index is mutated. A side that is
    // absent (the path was deleted there) maps to None, which resolves to a
    // delete rather than to an empty file.
    let mut sides: HashMap<String, (Option<Oid>, Option<Oid>)> = HashMap::new();
    {
        let conflicts =
            index.conflicts().map_err(|e| ToolError::internal(format!("read conflicts: {e}")))?;
        for c in conflicts {
            let c = c.map_err(|e| ToolError::internal(format!("read conflict entry: {e}")))?;
            let path = entry_path(&c.our)
                .or_else(|| entry_path(&c.their))
                .or_else(|| entry_path(&c.ancestor));
            let Some(path) = path else { continue };
            sides.insert(path, (c.our.map(|e| e.id), c.their.map(|e| e.id)));
        }
    }

    for (path, resolution) in resolutions {
        let rel = path.trim_start_matches('/');
        let blob = match resolution {
            Resolution::Strategy(Side::Ours) => sides.get(path).and_then(|s| s.0),
            Resolution::Strategy(Side::Theirs) => sides.get(path).and_then(|s| s.1),
            Resolution::Content(text) => Some(
                repo.blob(text.as_bytes())
                    .map_err(|e| ToolError::internal(format!("write resolved blob: {e}")))?,
            ),
        };
        index
            .conflict_remove(Path::new(rel))
            .map_err(|e| ToolError::internal(format!("clear conflict '{path}': {e}")))?;
        match blob {
            Some(oid) => index
                .add(&resolved_entry(rel, oid))
                .map_err(|e| ToolError::internal(format!("stage '{path}': {e}")))?,
            // Both sides agreed the path is gone, or the named side deleted it.
            None => {
                let _ = index.remove_path(Path::new(rel));
            }
        }
    }

    if index.has_conflicts() {
        return Err(ToolError::internal(
            "the resolved index still carries conflicts: the conflict set and the recorded \
             resolutions disagree",
        ));
    }
    index.write_tree_to(repo).map_err(|e| ToolError::internal(format!("write merged tree: {e}")))
}

/// A stage 0 index entry for a resolved path. Every field but the path, the
/// oid and the mode is zero: the index is in memory and never compared against
/// a working tree, so stat data would be noise.
fn resolved_entry(rel: &str, oid: Oid) -> git2::IndexEntry {
    git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: 0o100_644,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: oid,
        flags: 0,
        flags_extended: 0,
        path: rel.as_bytes().to_vec(),
    }
}

/// [`collect_conflicts`] for a caller that produced the index itself, so a
/// rebase step reports its conflicts through the very same reader `git.merge`
/// uses (FR-NEW-170).
pub fn conflicts_of_index(repo: &Repository, index: &git2::Index) -> Result<Vec<ConflictFile>> {
    collect_conflicts(repo, index)
}

/// Read every conflicting path out of a merged index, sorted ascending by
/// path so a caller sees a stable order whatever libgit2's internal order is.
fn collect_conflicts(repo: &Repository, index: &git2::Index) -> Result<Vec<ConflictFile>> {
    let conflicts =
        index.conflicts().map_err(|e| ToolError::internal(format!("read conflicts: {e}")))?;
    // Keyed by path: libgit2 can report the same path twice for a type change
    // (a delete entry plus an add entry), and the response carries one element
    // per path.
    let mut by_path: HashMap<String, ConflictFile> = HashMap::new();
    let staged = staged_paths(index);
    for c in conflicts {
        let c = c.map_err(|e| ToolError::internal(format!("read conflict entry: {e}")))?;
        let Some(path) = conflict_path(&c) else { continue };

        let ours = side(repo, &c.our);
        let theirs = side(repo, &c.their);
        let base = side(repo, &c.ancestor);
        // Binary on any present side means no side reports text: half decoded
        // content is worse than an explicit marker.
        let binary = [&c.our, &c.their, &c.ancestor].iter().any(|e| is_binary(repo, e));
        let type_change = type_change(&c, &staged, &path);
        let file = ConflictFile {
            path,
            ours: if binary { blank(&ours) } else { ours },
            theirs: if binary { blank(&theirs) } else { theirs },
            base: if binary { blank(&base) } else { base },
            binary,
            type_change,
        };
        by_path.insert(file.path.clone(), file);
    }
    let mut out: Vec<ConflictFile> = by_path.into_values().collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn entry_path(entry: &Option<git2::IndexEntry>) -> Option<String> {
    let e = entry.as_ref()?;
    Some(format!("/{}", String::from_utf8_lossy(&e.path)))
}

/// The absolute path a conflict is about, whichever of its three stages
/// carries it.
fn conflict_path(c: &git2::IndexConflict) -> Option<String> {
    entry_path(&c.our).or_else(|| entry_path(&c.their)).or_else(|| entry_path(&c.ancestor))
}

/// Why a conflicting path refuses a literal `content` resolution, or None
/// when it accepts one (FR-NEW-182, FR-NEW-183). The merge is replayed rather
/// than cached for the same reason [`resolve_merged_tree`] replays it: the
/// paused operation survives a restart and its only stored inputs are two
/// commits. Complexity is libgit2's merge, O(paths changed on either side),
/// plus O(bytes) of the conflicting blobs for the binary test; no content is
/// cloned or decoded.
pub fn conflict_restrictions(
    repo: &Repository,
    ours: &git2::Commit<'_>,
    theirs: &git2::Commit<'_>,
) -> Result<BTreeMap<String, &'static str>> {
    let index = repo
        .merge_commits(ours, theirs, None)
        .map_err(|e| ToolError::internal(format!("merge commits: {e}")))?;
    let staged = staged_paths(&index);
    let conflicts =
        index.conflicts().map_err(|e| ToolError::internal(format!("read conflicts: {e}")))?;
    let mut out = BTreeMap::new();
    for c in conflicts {
        let c = c.map_err(|e| ToolError::internal(format!("read conflict entry: {e}")))?;
        let Some(path) = conflict_path(&c) else { continue };
        // Binary is reported first: it is the stronger statement about the
        // bytes, and a binary path that also changed type still cannot carry
        // its content in a JSON string.
        let reason = if [&c.our, &c.their, &c.ancestor].iter().any(|e| is_binary(repo, e)) {
            "binary"
        } else if type_change(&c, &staged, &path) {
            "type change"
        } else {
            continue;
        };
        out.insert(path, reason);
    }
    Ok(out)
}

/// Every path the merged index holds, sorted. Needed because libgit2 reports
/// the side that replaced a file with a directory as an ABSENT side, not as a
/// mode difference: the directory shows up only as the entries under it.
fn staged_paths(index: &git2::Index) -> Vec<String> {
    let mut paths: Vec<String> =
        index.iter().map(|e| String::from_utf8_lossy(&e.path).into_owned()).collect();
    paths.sort();
    paths
}

/// True when some staged entry lives under `path/`, which is how a file to
/// directory type change is visible in the merged index.
fn became_directory(staged: &[String], path: &str) -> bool {
    let prefix = format!("{}/", path.trim_start_matches('/'));
    let at = staged.partition_point(|p| p.as_str() < prefix.as_str());
    staged.get(at).is_some_and(|p| p.starts_with(&prefix))
}

/// One side's `exists` and `content`. Content that is not valid UTF-8 reports
/// as binary through [`is_binary`], so the lossy branch here is unreachable
/// for a text file and only guards against a torn blob read.
fn side(repo: &Repository, entry: &Option<git2::IndexEntry>) -> ConflictSide {
    let Some(e) = entry else { return ConflictSide::absent() };
    match repo.find_blob(e.id) {
        Ok(b) => ConflictSide {
            exists: true,
            content: Some(String::from_utf8_lossy(b.content()).into_owned()),
        },
        Err(_) => ConflictSide::absent(),
    }
}

/// `exists` is preserved, `content` is dropped: a binary side exists but its
/// bytes are omitted rather than corrupted (FR-NEW-170).
fn blank(s: &ConflictSide) -> ConflictSide {
    ConflictSide { exists: s.exists, content: None }
}

fn is_binary(repo: &Repository, entry: &Option<git2::IndexEntry>) -> bool {
    let Some(e) = entry else { return false };
    match repo.find_blob(e.id) {
        Ok(b) => b.is_binary() || std::str::from_utf8(b.content()).is_err(),
        Err(_) => false,
    }
}

/// True when the sides disagree about what kind of entry the path is: a file
/// against a symlink or a submodule, which differ by mode, or a file against
/// a directory, which does not appear in the modes at all. DEC-909 surfaces
/// either as a conflict carrying this marker rather than refusing the
/// operation.
fn type_change(c: &git2::IndexConflict, staged: &[String], path: &str) -> bool {
    let kinds: HashSet<u32> = [&c.our, &c.their, &c.ancestor]
        .iter()
        .filter_map(|e| e.as_ref().map(|e| e.mode & 0o170_000))
        .collect();
    kinds.len() > 1 || became_directory(staged, path)
}

// ── the volume side: quota, cleanliness, atomic apply ───────────────────────

/// One path a tree to tree delta touches: written with new content, or
/// removed. Renames are never reported here: `diff_tree_to_tree` without
/// `find_similar` reports a rename as a delete plus an add, which is exactly
/// how they end up represented as two separate [`TreeChange`] values.
pub enum TreeChange {
    Write { path: String, oid: Oid },
    Delete { path: String },
}

/// The paths a move from `old_tree` to `new_tree` touches, without rename
/// detection (`diff_tree_to_tree`'s default: a replaced file surfaces as a
/// delete of the old path plus an add of the new one, which is exactly the
/// shape [`apply_changes_atomically`] needs).
pub fn diff_tree_changes(
    repo: &Repository,
    old_tree: &Tree<'_>,
    new_tree: &Tree<'_>,
) -> Result<Vec<TreeChange>> {
    let diff = repo
        .diff_tree_to_tree(Some(old_tree), Some(new_tree), None)
        .map_err(|e| ToolError::internal(format!("diff: {e}")))?;
    let mut changes = Vec::new();
    for delta in diff.deltas() {
        match delta.status() {
            git2::Delta::Deleted => {
                if let Some(p) = delta.old_file().path() {
                    changes.push(TreeChange::Delete { path: format!("/{}", p.to_string_lossy()) });
                }
            }
            git2::Delta::Added | git2::Delta::Modified | git2::Delta::Typechange => {
                if let Some(p) = delta.new_file().path() {
                    changes.push(TreeChange::Write {
                        path: format!("/{}", p.to_string_lossy()),
                        oid: delta.new_file().id(),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(changes)
}

/// Charge the quota, then apply, giving the charge back when the apply refused
/// (FR-NEW-184, FR-NEW-185).
///
/// The charge must come first, so an over-quota operation is rejected before a
/// byte moves. The refund is what makes the pair all-or-nothing including its
/// cost: pass 1 of [`apply_changes_atomically`] pre-checks every target, so a
/// refusal there provably wrote nothing and must not consume quota.
pub async fn charge_and_apply(
    operation: &str,
    repo: &Repository,
    safety: &SafetyManager,
    person: &str,
    client: &VolumeClient,
    changes: &[TreeChange],
) -> Result<()> {
    let charged = charge_quota(repo, safety, person, client, changes)?;
    match apply_changes_atomically(operation, client, repo, changes).await {
        Ok(()) => Ok(()),
        Err(e) => {
            safety.refund_write(person, &client.project_id, charged);
            Err(e)
        }
    }
}

/// FR-NEW-069: the single authority on the basis of a combine operation's
/// write-quota charge, fast-forward or merged. Sums only the size of the blobs
/// actually written (`TreeChange::Write`); a deleted path never adds bytes.
/// Returns what was charged, so a caller that fails to apply can give it back.
pub fn charge_quota(
    repo: &Repository,
    safety: &SafetyManager,
    person: &str,
    client: &VolumeClient,
    changes: &[TreeChange],
) -> Result<i64> {
    let charge_bytes: i64 = changes
        .iter()
        .filter_map(|c| match c {
            TreeChange::Write { oid, .. } => repo.find_blob(*oid).ok().map(|b| b.size() as i64),
            TreeChange::Delete { .. } => None,
        })
        .sum();
    safety.charge_write(person, &client.project_id, charge_bytes)?;
    Ok(charge_bytes)
}

/// Apply every change to the volume, or none of them (FR-NEW-035, DEC-023).
/// Every write target is checked for writability before any byte moves
/// (pass 1), so a failure is caught pre-commit; only once every check passes
/// does anything actually mutate (pass 2), and only after that does the caller
/// advance the ref.
pub async fn apply_changes_atomically(
    operation: &str,
    client: &VolumeClient,
    repo: &Repository,
    changes: &[TreeChange],
) -> Result<()> {
    let delete_paths: HashSet<&str> = changes
        .iter()
        .filter_map(|c| match c {
            TreeChange::Delete { path } => Some(path.as_str()),
            TreeChange::Write { .. } => None,
        })
        .collect();

    // Pass 1: every write target must be provably writable before the first
    // byte moves.
    for change in changes {
        if let TreeChange::Write { path, .. } = change {
            check_path_writable(operation, client, path, &delete_paths).await?;
        }
    }

    // Pass 2: apply. Deletes first, so a path changing from a file to a
    // directory of the same name (or the reverse, where the pre-check above
    // already refused it) lands in the right final state.
    for change in changes {
        if let TreeChange::Delete { path } = change {
            client.delete_file(path).await?;
        }
    }
    for change in changes {
        if let TreeChange::Write { path, oid } = change {
            let blob =
                repo.find_blob(*oid).map_err(|e| ToolError::internal(format!("read blob: {e}")))?;
            if let Some(parent) = crate::util::PosixPath::parent_of(path)
                && parent != "/"
            {
                client.makedirs(&parent, true).await?;
            }
            client.write_bytes_atomic(path, blob.content()).await?;
        }
    }
    Ok(())
}

/// `path` itself must not already be a directory (a directory-to-file
/// typechange is not supported by this apply: it would need a whole-subtree
/// delete, which this atomic apply deliberately does not attempt), and every
/// ancestor directory of `path` must be absent, already a directory, or itself
/// scheduled for deletion in this same operation (a file-to-directory
/// typechange, which the delete side of the diff already clears out of the
/// way).
async fn check_path_writable(
    operation: &str,
    client: &VolumeClient,
    path: &str,
    delete_paths: &HashSet<&str>,
) -> Result<()> {
    if client.is_dir(path).await? {
        return Err(ToolError::invalid_argument(format!(
            "{operation} cannot write '{path}': it already exists as a directory in the volume"
        )));
    }
    let Some(parent) = crate::util::PosixPath::parent_of(path) else { return Ok(()) };
    let mut probe = String::new();
    for seg in parent.trim_matches('/').split('/').filter(|s| !s.is_empty()) {
        probe.push('/');
        probe.push_str(seg);
        if !delete_paths.contains(probe.as_str()) && client.is_file(&probe).await? {
            return Err(ToolError::invalid_argument(format!(
                "{operation} cannot write '{path}': '{probe}' already exists as a file"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conflict_response_serializes_exactly_the_contract_keys() {
        let r = ConflictResponse::new(
            GitOpType::Merge,
            "gitproj:merge:1".to_string(),
            Some("feature".to_string()),
            None,
            vec![ConflictFile {
                path: "/a.txt".to_string(),
                ours: ConflictSide { exists: true, content: Some("a\n".to_string()) },
                theirs: ConflictSide { exists: true, content: Some("b\n".to_string()) },
                base: ConflictSide::absent(),
                binary: false,
                type_change: false,
            }],
        );
        let v = r.to_value();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "status",
                "operation",
                "operation_id",
                "source_ref",
                "current_step",
                "total_steps",
                "conflicts",
                "continue_with",
                "abort_with"
            ]
            .iter()
            .collect::<Vec<_>>()
        );
        let c = &v["conflicts"][0];
        let ckeys: Vec<&String> = c.as_object().unwrap().keys().collect();
        assert_eq!(
            ckeys,
            ["path", "ours", "theirs", "base", "binary", "type_change"].iter().collect::<Vec<_>>()
        );
        let skeys: Vec<&String> = c["base"].as_object().unwrap().keys().collect();
        assert_eq!(skeys, ["exists", "content"].iter().collect::<Vec<_>>());
        // The four field names the specification's audit found drifting.
        for forbidden in ["conflicting_paths", "resolve_with", "step", "target_ref"] {
            assert!(v.get(forbidden).is_none(), "{forbidden} must be emitted by no tool");
        }
        assert_eq!(v["current_step"], Value::Null, "single step operations report null");
        assert_eq!(v["continue_with"], "git.merge_resolve");
        assert_eq!(v["abort_with"], "git.merge_abort");
    }

    #[test]
    fn the_merge_response_serializes_exactly_the_contract_keys() {
        let v = MergeResponse {
            status: "merged",
            merge_commit: None,
            fast_forward: true,
            squashed: false,
            files_changed: 1,
        }
        .to_value();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["status", "merge_commit", "fast_forward", "squashed", "files_changed"]
                .iter()
                .collect::<Vec<_>>()
        );
        for forbidden in ["commit_sha", "parents", "conflicts", "operation_id"] {
            assert!(v.get(forbidden).is_none(), "{forbidden} must be emitted by no tool");
        }
    }
}
