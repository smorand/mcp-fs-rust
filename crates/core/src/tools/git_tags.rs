//! Tag engines (SPEC-0052): `git.tag_create` and the `git.tags` listing.
//!
//! A tag is one `git_refs` row named `refs/tags/<name>` whose target is the
//! commit (a plain tag) or a tag object (an annotated tag), exactly like a
//! branch is a `refs/heads/` row: no second store, no migration. The helpers
//! shared with every other git tool (membership, the paused operation guard,
//! hydration, the blocking pool) live in [`super::git`] and are reused here
//! rather than copied, so a tag refusal reads the same as a branch refusal.

use crate::errors::{Result, ToolError};
use crate::git::{GitRepoEntry, GitRepoStore};
use crate::safety::SafetyManager;
use crate::tools::git::{
    hydrate, on_git_thread, open, parse_oid, reject_if_operation_in_progress, validate_ref_name,
};
use crate::tools::registry_support::ToolCtx;
use chrono::{DateTime, Utc};
use git2::{ObjectType, Oid, Repository};
use serde_json::{Value, json};
use std::sync::Arc;

const TAGS_PREFIX: &str = "refs/tags/";

/// FR-NEW-021: a commit prefix shorter than this is never resolved, so a short
/// text cannot silently land on whichever commit happens to match today.
const MIN_PREFIX_LEN: usize = 7;

/// The full length of a sha1 id in hex.
const FULL_ID_LEN: usize = 40;

/// `git.tag_create`: refusals come in the order of FR-NEW-020 because that is
/// the order of the calls: membership (`open`), paused operation, name, then
/// inside [`tag_create`] no commit, target, existing name.
pub(crate) async fn tool_tag_create(
    ctx: ToolCtx,
    a: crate::mcp::server::GitTagCreateArgs,
    g: Option<Arc<GitRepoStore>>,
) -> Result<Value> {
    let mount_id = a.mount_id.clone();
    let entry = open(&ctx, &mount_id, g).await?;
    reject_if_operation_in_progress(&entry, &mount_id, "git.tag_create").await?;
    validate_ref_name(&a.name, "tag")?;
    let safety = ctx.state.safety.clone();
    let person = ctx.person.clone();
    let target = Some(a.target).filter(|t| !t.is_empty());
    on_git_thread(move || async move {
        let request = TagRequest {
            name: &a.name,
            target: target.as_deref(),
            message: &a.message,
            overwrite: a.overwrite,
        };
        tag_create(&entry, &safety, &person, &mount_id, request).await
    })
    .await
}

/// `git.tags`: listing runs during a paused operation too, since reading a
/// tag changes nothing (FR-NEW-006 only guards creation and deletion).
pub(crate) async fn tool_tags(
    ctx: ToolCtx,
    a: crate::mcp::server::MountOnlyArgs,
    g: Option<Arc<GitRepoStore>>,
) -> Result<Value> {
    let mount_id = a.mount_id.clone();
    let entry = open(&ctx, &mount_id, g).await?;
    on_git_thread(move || async move {
        let tags = list_tags(&entry).await?;
        Ok(json!({"mount_id": mount_id, "tags": tags}))
    })
    .await
}

/// The caller's choices for one creation; an empty `message` means a plain tag.
struct TagRequest<'a> {
    name: &'a str,
    target: Option<&'a str>,
    message: &'a str,
    overwrite: bool,
}

/// DEC-009: the write lock covers the existence check and the ref write
/// together, so of two concurrent creations of one name exactly one passes the
/// check; the other then sees the winner's row and gets the FR-NEW-003 refusal.
/// Every refusal happens before the first write, which keeps a refused call
/// free of a row and of an audit entry.
async fn tag_create(
    entry: &GitRepoEntry,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    request: TagRequest<'_>,
) -> Result<Value> {
    let TagRequest { name, target, message, overwrite } = request;
    let tag_ref = format!("{TAGS_PREFIX}{name}");
    let _write = entry.write_lock.lock().await;

    // Hydrated before any lookup: target resolution reads commits and tag
    // objects, and the on disk ref written below needs its commit present.
    let repo = entry.repo.lock().await;
    hydrate(entry, &repo).await?;

    let commit = match target {
        None => head_commit(entry)
            .await?
            .ok_or_else(|| ToolError::not_found("Cannot tag: this project has no commit yet."))?,
        Some(t) => resolve_target(entry, &repo, t).await?.ok_or_else(|| {
            ToolError::not_found(format!("Cannot tag '{t}': no such branch, tag or commit."))
        })?,
    };

    let previous = entry.db.get_ref(&tag_ref).await?.map(|r| r.target);
    if previous.is_some() && !overwrite {
        return Err(ToolError::no_clobber(format!(
            "Tag '{name}' already exists. Choose another name or confirm overwriting it."
        )));
    }

    let sha = commit.to_string();
    let (kind, object) = if message.is_empty() {
        ("lightweight", commit)
    } else {
        let tag_oid = write_tag_object(&repo, person, name, commit, message)?;
        // The tag object must be in the authoritative blob store before the
        // ref row names it, or a reader could find a ref to nothing.
        entry.objects.import_from_repo(&repo).await?;
        ("annotated", tag_oid)
    };
    let object_sha = object.to_string();
    entry.db.set_ref(&tag_ref, &object_sha, false).await?;
    // Keep the on disk refs in step, the same way git.branch_create does. The
    // index row is the source of truth; libgit2 refuses a few names the branch
    // rules accept (DEC-019), and that refusal must not undo a valid tag.
    let _ = repo.reference(&tag_ref, object, true, "mcp-fs git.tag_create");

    // The previous target of a plain tag is its commit; peeling also covers a
    // tag that was annotated before, so the line always names a commit.
    let previous_commit =
        previous.as_deref().map(|p| peel_to_commit(&repo, p).unwrap_or_else(|| p.to_string()));
    let (action, detail) = match &previous_commit {
        Some(old) => ("git.tag_overwrite", format!("overwrote tag '{name}' at {sha} (was {old})")),
        None => ("git.tag_create", format!("created tag '{name}' at {sha}")),
    };
    safety.record_audit(person, &entry.project_id, action, "/", &detail);

    let mut out = json!({
        "mount_id": mount_id,
        "name": name,
        "kind": kind,
        "sha": sha,
        "object": object_sha,
        "overwritten": previous_commit.is_some(),
    });
    if let (Some(old), Some(map)) = (previous_commit, out.as_object_mut()) {
        map.insert("previous_sha".to_string(), Value::String(old));
    }
    Ok(out)
}

/// DEC-005, DEC-006: the tag object is assembled by hand and written through
/// the object database, because `Repository::tag` validates names more
/// strictly than the branch rules and would also normalise the message, which
/// FR-NEW-002 forbids (the message is kept byte for byte, so no prettify).
/// The tagger is built like a commit author: name before `@`, email the id.
fn write_tag_object(
    repo: &Repository,
    person: &str,
    name: &str,
    commit: Oid,
    message: &str,
) -> Result<Oid> {
    // A header line ends at a newline and brackets delimit the email, so a
    // person id carrying any of them could forge or break the header.
    let clean = |s: &str| s.chars().filter(|c| !matches!(c, '\n' | '<' | '>')).collect::<String>();
    let tagger_name = clean(person.split('@').next().unwrap_or(person));
    let tagger_email = clean(person);
    let body = format!(
        "object {commit}\ntype commit\ntag {name}\ntagger {tagger_name} <{tagger_email}> {} +0000\n\n{message}",
        Utc::now().timestamp()
    );
    let odb = repo.odb().map_err(|e| ToolError::internal(format!("tag object store: {e}")))?;
    odb.write(ObjectType::Tag, body.as_bytes())
        .map_err(|e| ToolError::internal(format!("write tag object: {e}")))
}

/// The commit HEAD stands on, or `None` when the project has no commit: a
/// fresh repository's HEAD names a branch that has no row yet.
async fn head_commit(entry: &GitRepoEntry) -> Result<Option<Oid>> {
    let Some(head) = entry.db.get_ref("HEAD").await? else { return Ok(None) };
    let sha = if head.symbolic {
        match entry.db.get_ref(&head.target).await? {
            Some(r) => r.target,
            None => return Ok(None),
        }
    } else {
        head.target
    };
    parse_oid(&sha).map(Some)
}

/// DEC-007, FR-NEW-021: a branch of that exact name, then a tag of that exact
/// name (peeled to its commit), then a full commit id, then the only commit
/// whose id begins with the text when it is at least 7 characters. `None` is
/// an unknown target, which covers an ambiguous prefix too: guessing between
/// two commits would tag the wrong one silently.
///
/// The existing `resolve_ref` is not reused: it returns any hex text as a sha
/// without checking the commit exists.
async fn resolve_target(entry: &GitRepoEntry, repo: &Repository, t: &str) -> Result<Option<Oid>> {
    for prefix in ["refs/heads/", TAGS_PREFIX] {
        if let Some(row) = entry.db.get_ref(&format!("{prefix}{t}")).await?
            && let Some(commit) = peel_to_commit(repo, &row.target)
        {
            return parse_oid(&commit).map(Some);
        }
    }
    if t.len() < MIN_PREFIX_LEN
        || t.len() > FULL_ID_LEN
        || !t.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Ok(None);
    }
    if t.len() == FULL_ID_LEN {
        let Ok(oid) = Oid::from_str(t) else { return Ok(None) };
        return Ok(repo.find_commit(oid).ok().map(|c| c.id()));
    }
    // libgit2 reports an ambiguous prefix as an error, which is exactly the
    // unknown target FR-NEW-021 asks for, so every failure maps to `None`.
    Ok(repo
        .find_object_by_prefix(t, None)
        .ok()
        .filter(|o| o.kind() == Some(ObjectType::Commit))
        .map(|o| o.id()))
}

/// The commit an object id peels to, or `None` when it is absent or names no
/// commit (a tag on a tree, say).
fn peel_to_commit(repo: &Repository, sha: &str) -> Option<String> {
    let oid = Oid::from_str(sha).ok()?;
    let object = repo.find_object(oid, None).ok()?;
    object.peel_to_commit().ok().map(|c| c.id().to_string())
}

/// DEC-006, DEC-010: every `refs/tags/` row, `sha` peeled to its commit and
/// `object` the row's own target, in ascending byte order of name.
///
/// Cost: one ref listing query, then for each of the n tags one object lookup
/// and peel, plus an O(n log n) sort; the repository is hydrated only when at
/// least one tag exists, so a project without tags never touches libgit2.
async fn list_tags(entry: &GitRepoEntry) -> Result<Vec<Value>> {
    let mut rows: Vec<(String, String)> = entry
        .db
        .list_refs()
        .await?
        .into_iter()
        .filter(|r| !r.symbolic && r.name.starts_with(TAGS_PREFIX))
        .map(|r| (r.name, r.target))
        .collect();
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    // `String` ordering is byte ordering, which is what FR-MOD-001 asks for;
    // sorting the full ref sorts the names since they share one prefix.
    rows.sort_unstable();

    let repo = entry.repo.lock().await;
    hydrate(entry, &repo).await?;
    Ok(rows.into_iter().map(|(full_ref, target)| tag_entry(&repo, full_ref, target)).collect())
}

fn tag_entry(repo: &Repository, full_ref: String, target: String) -> Value {
    let name = full_ref.strip_prefix(TAGS_PREFIX).unwrap_or(&full_ref).to_string();
    let annotated = Oid::from_str(&target)
        .ok()
        .and_then(|oid| repo.find_object(oid, None).ok())
        .is_some_and(|o| o.kind() == Some(ObjectType::Tag));
    // A target the store cannot peel (an object never imported) is still
    // listed, under its own id, rather than hiding the tag or failing the list.
    let sha = peel_to_commit(repo, &target).unwrap_or_else(|| target.clone());
    let kind = if annotated { "annotated" } else { "lightweight" };
    let mut entry = json!({
        "name": name,
        "full_ref": full_ref,
        "sha": sha,
        "object": target,
        "kind": kind,
    });
    if let (true, Some(map)) = (annotated, entry.as_object_mut()) {
        annotation_details(repo, &target, map);
    }
    entry
}

/// FR-MOD-001: message exactly as stored, tagger name and email, creation
/// time in UTC. A tag object that cannot be parsed adds nothing rather than
/// failing the listing, the same leniency as an unpeelable target.
fn annotation_details(repo: &Repository, target: &str, map: &mut serde_json::Map<String, Value>) {
    let Some(tag) = Oid::from_str(target).ok().and_then(|oid| repo.find_tag(oid).ok()) else {
        return;
    };
    map.insert(
        "message".to_string(),
        Value::String(
            String::from_utf8_lossy(tag.message_bytes().unwrap_or_default()).into_owned(),
        ),
    );
    let Some(tagger) = tag.tagger() else { return };
    map.insert(
        "tagger".to_string(),
        Value::String(String::from_utf8_lossy(tagger.name_bytes()).into_owned()),
    );
    map.insert(
        "tagger_email".to_string(),
        Value::String(String::from_utf8_lossy(tagger.email_bytes()).into_owned()),
    );
    if let Some(at) = DateTime::<Utc>::from_timestamp(tagger.when().seconds(), 0) {
        map.insert(
            "tagged_at".to_string(),
            Value::String(at.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        );
    }
}
