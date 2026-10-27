//! Tag engines (SPEC-0052): `git.tag_create`, `git.tag_delete`, the
//! `git.tags` listing, `git.remote_push_tag` and `git.remote_delete_tag`
//! (SPEC-0053).
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
    hydrate, on_git_thread, open, parse_oid, reject_if_operation_in_progress,
    resolve_clone_credential, validate_ref_name,
};
use crate::tools::registry_support::ToolCtx;
use chrono::{DateTime, Utc};
use git2::{ObjectType, Oid, Repository};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// `git.tag_delete`: membership, then the paused operation, then inside
/// [`tag_delete`] the unknown tag (FR-NEW-020). The name is not validated: a
/// name no rule accepts can never exist, so it gets the unknown tag refusal.
pub(crate) async fn tool_tag_delete(
    ctx: ToolCtx,
    a: crate::mcp::server::GitTagDeleteArgs,
    g: Option<Arc<GitRepoStore>>,
) -> Result<Value> {
    let mount_id = a.mount_id.clone();
    let entry = open(&ctx, &mount_id, g).await?;
    reject_if_operation_in_progress(&entry, &mount_id, "git.tag_delete").await?;
    let safety = ctx.state.safety.clone();
    let person = ctx.person.clone();
    on_git_thread(
        move || async move { tag_delete(&entry, &safety, &person, &mount_id, &a.name).await },
    )
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

/// DEC-009, FR-NEW-016: the existence check and the removal share the write
/// lock, so of two concurrent deletions of one tag the second finds no row and
/// gets the FR-NEW-005 refusal. Only the tag ref goes: commits, other refs and
/// the remote are never touched, and the tag object stays in the store like
/// any unreferenced object.
///
/// Cost: O(1) ref lookups and one row delete; the repository is hydrated (O(n)
/// in stored objects, the same as every other tag tool) only for an existing
/// tag, since peeling an annotated tag needs its object.
async fn tag_delete(
    entry: &GitRepoEntry,
    safety: &SafetyManager,
    person: &str,
    mount_id: &str,
    name: &str,
) -> Result<Value> {
    let tag_ref = format!("{TAGS_PREFIX}{name}");
    let _write = entry.write_lock.lock().await;

    let Some(row) = entry.db.get_ref(&tag_ref).await? else {
        return Err(ToolError::not_found(format!("Tag '{name}' does not exist.")));
    };

    let repo = entry.repo.lock().await;
    hydrate(entry, &repo).await?;
    // The audit and the answer name a commit, so an annotated tag is peeled;
    // an unpeelable target is still deleted and reported under its own id.
    let sha = peel_to_commit(&repo, &row.target).unwrap_or(row.target);

    entry.db.delete_ref(&tag_ref).await?;
    // Keep the on disk refs in step, the same way git.branch_delete does; an
    // absent on disk ref (DEC-019 names libgit2 refuses) is already in step.
    if let Ok(mut on_disk) = repo.find_reference(&tag_ref) {
        let _ = on_disk.delete();
    }

    safety.record_audit(
        person,
        &entry.project_id,
        "git.tag_delete",
        "/",
        &format!("deleted tag '{name}' (was at {sha})"),
    );
    Ok(json!({"mount_id": mount_id, "name": name, "sha": sha}))
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

/// DEC-017: two tag targets are the same tag when both are plain on one
/// commit, or both annotated on one final commit with the same message;
/// tagger and time do not count, and a plain tag never equals an annotated
/// one. Both objects must be in `repo`; a missing one is an error, not a
/// difference, because reconcile runs only after the fetch brought it in.
pub(crate) fn tags_identical(repo: &Repository, a: Oid, b: Oid) -> Result<bool> {
    if a == b {
        return Ok(true);
    }
    let find = |oid: Oid| {
        repo.find_object(oid, None)
            .map_err(|e| ToolError::internal(format!("tag target {oid} unreadable: {e}")))
    };
    let (a, b) = (find(a)?, find(b)?);
    match (a.as_tag(), b.as_tag()) {
        (Some(ta), Some(tb)) => {
            let peel = |o: &git2::Object<'_>| o.peel_to_commit().ok().map(|c| c.id());
            Ok(peel(&a).is_some()
                && peel(&a) == peel(&b)
                && ta.message_bytes() == tb.message_bytes())
        }
        // Two plain tags are equal only on the same object, checked above.
        _ => Ok(false),
    }
}

/// DEC-012: record each incoming tag the project does not have, keep every
/// local tag of the same name untouched, and report both lists sorted. A tag
/// identical to the local one (DEC-017) is in neither list. Must run after the
/// fetched objects are imported into the blob store, so no row ever names an
/// object the store lacks.
///
/// Cost: one ref listing, then for n incoming tags O(n) map lookups, one
/// comparison per name clash and one row write per added tag; O(n) space.
pub(crate) async fn reconcile_incoming_tags(
    entry: &GitRepoEntry,
    repo: &Repository,
    incoming: &[crate::git::remote::IncomingTag],
) -> Result<(Vec<String>, Vec<String>)> {
    let local: std::collections::HashMap<String, String> = entry
        .db
        .list_refs()
        .await?
        .into_iter()
        .filter(|r| !r.symbolic && r.name.starts_with(TAGS_PREFIX))
        .map(|r| (r.name, r.target))
        .collect();
    let (mut added, mut kept_local) = (Vec::new(), Vec::new());
    for tag in incoming {
        let tag_ref = format!("{TAGS_PREFIX}{}", tag.name);
        match local.get(&tag_ref) {
            None => {
                entry.db.set_ref(&tag_ref, &tag.oid.to_string(), false).await?;
                // Keep the on disk refs in step, as git.tag_create does; a
                // name libgit2 refuses (DEC-019) must not undo the row.
                let _ = repo.reference(&tag_ref, tag.oid, true, "mcp-fs git.remote_fetch");
                added.push(tag.name.clone());
            }
            Some(target) => {
                if !tags_identical(repo, parse_oid(target)?, tag.oid)? {
                    kept_local.push(tag.name.clone());
                }
            }
        }
    }
    added.sort_unstable();
    kept_local.sort_unstable();
    Ok((added, kept_local))
}

/// `git.remote_push_tag`: refusals come in the order of FR-NEW-020 because
/// that is the order of the calls: membership (`open`), unknown tag, no
/// remote, credential, then inside [`publish_tag`] the connection and the
/// remote checks. None of them writes an audit entry (DEC-016).
pub(crate) async fn tool_remote_push_tag(
    ctx: ToolCtx,
    a: crate::mcp::server::GitRemotePushTagArgs,
    g: Option<Arc<GitRepoStore>>,
    t: Option<Arc<crate::git::OAuthTokenStore>>,
) -> Result<Value> {
    let entry = open(&ctx, &a.mount_id, g).await?;
    if entry.db.get_ref(&format!("{TAGS_PREFIX}{}", a.tag)).await?.is_none() {
        return Err(unknown_tag(&a.tag));
    }
    let Some(url) =
        entry.db.list_remotes().await?.into_iter().find(|(n, _)| *n == a.remote).map(|(_, u)| u)
    else {
        return Err(ToolError::not_found(format!(
            "Cannot publish tag '{}': this project tracks no remote.",
            a.tag
        )));
    };
    let (token, auth) = resolve_clone_credential(&ctx, &t, &url).await?;
    let timeout_secs = ctx.state.config.git.remote_timeout_secs;
    let target = PublishTarget {
        mount_id: &a.mount_id,
        person: &ctx.person,
        remote: &a.remote,
        tag: &a.tag,
        url: &url,
        token,
        auth: &auth,
        timeout_secs,
        overwrite: a.overwrite,
        expected_remote_commit: &a.expected_remote_commit,
    };
    publish_tag(entry, ctx.state.safety.clone(), target).await
}

/// Where and how one tag is published: everything [`publish_tag`] needs
/// once the remote URL and the credential are known.
pub(crate) struct PublishTarget<'a> {
    pub mount_id: &'a str,
    pub person: &'a str,
    pub remote: &'a str,
    pub tag: &'a str,
    pub url: &'a str,
    pub token: Option<String>,
    pub auth: &'a str,
    pub timeout_secs: u64,
    /// FR-NEW-008: replacing a differing remote tag was confirmed.
    pub overwrite: bool,
    /// DEC-018: the commit the requester believes the remote tag is at.
    pub expected_remote_commit: &'a str,
}

/// DEC-015, DEC-016, DEC-020: publish one tag once the remote and the
/// credential are known. Split from [`tool_remote_push_tag`] so a test can
/// reach it with a `file://` remote, which the registered tool refuses.
///
/// Under the write lock (DEC-009), one listing connection, then the checks,
/// then at most one push connection. The audit entry is written only when
/// the push really happened: an identical remote tag, a refusal and a
/// connection failure all leave the trail untouched (FR-NEW-012).
///
/// Cost: hydration, O(o) in stored objects as for every tag tool; then for
/// n advertised refs O(n) work plus one `graph_descendant_of` per distinct
/// advertised commit held locally, each O(d) in the history depth d, so
/// O(n + u d) time for u distinct tips and O(n) space. Nothing is fetched,
/// except one tag object when both tags are annotated on the same commit
/// (DEC-017), and a lease lost between listing and push costs one more
/// listing, O(n).
pub(crate) async fn publish_tag(
    entry: Arc<GitRepoEntry>,
    safety: Arc<SafetyManager>,
    target: PublishTarget<'_>,
) -> Result<Value> {
    let host = crate::git::remote::extract_host(target.url);
    let (remote, auth) = (target.remote, target.auth);
    crate::git::remote::trace_remote_operation(
        "git.remote_push_tag",
        remote,
        &host,
        auth,
        publish_inner(entry, safety, target, &host),
    )
    .await
}

async fn publish_inner(
    entry: Arc<GitRepoEntry>,
    safety: Arc<SafetyManager>,
    target: PublishTarget<'_>,
    host: &str,
) -> Result<Value> {
    let PublishTarget {
        mount_id,
        person,
        remote,
        tag,
        url,
        token,
        auth,
        timeout_secs,
        overwrite,
        expected_remote_commit,
    } = target;
    let tag_ref = format!("{TAGS_PREFIX}{tag}");
    // Acquired here, in the async caller, never inside the blocking closure,
    // so a deadline firing releases it (DRIFT-009), as for a branch push.
    let _write = entry.write_lock.lock().await;
    let Some(row) = entry.db.get_ref(&tag_ref).await? else {
        return Err(unknown_tag(tag));
    };
    let object = parse_oid(&row.target)?;
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let (thread_entry, tag_owned, url_owned) = (entry.clone(), tag.to_string(), url.to_string());
    let confirmed = overwrite.then(|| expected_remote_commit.to_string());
    let (commit, outcome) = crate::git::remote::with_remote_deadline(
        timeout_secs,
        host,
        on_git_thread(move || async move {
            let repo = thread_entry.repo.lock().await;
            hydrate(&thread_entry, &repo).await?;
            let publish = Publish { tag: &tag_owned, object, url: &url_owned, token, deadline };
            publish_on_repo(&repo, &publish, confirmed.as_deref())
        }),
    )
    .await?;

    let pushed = outcome != Published::UpToDate;
    if pushed {
        safety.record_audit(
            person,
            &entry.project_id,
            "git.tag_publish",
            "/",
            &format!("published tag '{tag}' at {commit}"),
        );
    }
    Ok(json!({
        "mount_id": mount_id,
        "tag": tag,
        "remote": remote,
        "created": outcome == Published::Created,
        "up_to_date": !pushed,
        "remote_sha": commit,
        "auth": auth,
    }))
}

/// What a publication did to the remote tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Published {
    Created,
    Replaced,
    UpToDate,
}

/// The local tag and the remote connection of one blocking publication.
struct Publish<'a> {
    tag: &'a str,
    object: Oid,
    url: &'a str,
    token: Option<String>,
    deadline: Instant,
}

/// The blocking half of a publication: list, check, then push. `confirmed`
/// is the stated remote commit when an overwrite was confirmed (DEC-018).
/// Returns the tag's commit and what happened.
fn publish_on_repo(
    repo: &Repository,
    publish: &Publish<'_>,
    confirmed: Option<&str>,
) -> Result<(String, Published)> {
    let Publish { tag, object, url, ref token, deadline } = *publish;
    let tag_ref = format!("{TAGS_PREFIX}{tag}");
    let commit = peel_to_commit(repo, &object.to_string()).ok_or_else(|| {
        ToolError::internal(format!("tag '{tag}' does not name a commit held by the project"))
    })?;
    let commit_oid = parse_oid(&commit)?;
    let advertised =
        crate::git::remote::list_advertised_refs(repo, url, token.clone(), Some(deadline))?;
    #[cfg(test)]
    after_listing::fire(url);

    if !commit_on_remote(repo, commit_oid, &advertised) {
        return Err(ToolError::invalid_argument(format!(
            "Cannot publish tag '{tag}': commit {} is not on the remote. Publish its branch first.",
            short7(&commit)
        )));
    }

    // DEC-018: the lease is the remote tag object seen in this one listing;
    // `None` means the remote has no such tag and the push is a creation.
    let mut lease = None;
    if let Some(remote_object) = advertised_oid(&advertised, &tag_ref) {
        let peeled_entry = advertised_oid(&advertised, &format!("{tag_ref}^{{}}"));
        let remote_commit = peeled_entry.unwrap_or(remote_object);
        let remote_annotated = peeled_entry.is_some();
        let local_annotated = object != commit_oid;
        // Identical without a transfer when the object is the same. Two
        // annotated tags on one commit may still carry one message under
        // another tagger or time, which only the remote's tag object can
        // tell; every other combination differs by kind or by commit.
        let identical = remote_object == object
            || (remote_annotated
                && local_annotated
                && remote_commit == commit_oid
                && probe_identical(repo, publish)?);
        if identical {
            return Ok((commit, Published::UpToDate));
        }
        let remote_commit = remote_commit.to_string();
        match confirmed {
            Some(stated) if !stated.trim().is_empty() => {
                if !stated_commit_matches(stated, &remote_commit) {
                    return Err(moved(tag, &remote_commit));
                }
            }
            _ => {
                return Err(ToolError::no_clobber(format!(
                    "Remote tag '{tag}' already exists at a different commit ({}). Confirm \
                     overwriting to replace it.",
                    short7(&remote_commit)
                )));
            }
        }
        lease = Some(remote_object);
    }

    // The push names the tag by ref, so the index row (the source of truth)
    // is written to the on disk ref first, as a branch push does. A name the
    // branch rules accept but git refuses as a reference (DEC-019) is
    // refused here, before the push connection, with the refusal a remote
    // would give. Checked rather than left to `reference`, which quietly
    // normalises `a//b` to `a/b` and would write a ref of another name.
    if !git2::Reference::is_valid_name(&tag_ref) {
        return Err(crate::git::remote::remote_refused_tag(tag, "invalid ref name"));
    }
    repo.reference(&tag_ref, object, true, "mcp-fs git.remote_push_tag")
        .map_err(|e| crate::git::remote::remote_refused_tag(tag, e.message()))?;
    let pushed = crate::git::remote::push_tag_to_remote(
        repo,
        url,
        tag,
        token.clone(),
        Some(deadline),
        lease,
    )?;
    if pushed == crate::git::remote::TagPush::LeaseLost {
        // The remote tag moved between the listing and the push, and the
        // push was aborted before anything was sent. One more listing names
        // where it went, so the refusal reads like case 5 of DEC-018.
        let now =
            crate::git::remote::list_advertised_refs(repo, url, token.clone(), Some(deadline))?;
        let peeled = advertised_oid(&now, &format!("{tag_ref}^{{}}"))
            .or_else(|| advertised_oid(&now, &tag_ref))
            .unwrap_or(Oid::ZERO_SHA1);
        return Err(moved(tag, &peeled.to_string()));
    }
    let outcome = if lease.is_some() { Published::Replaced } else { Published::Created };
    Ok((commit, outcome))
}

/// DEC-017: whether the remote annotated tag carries the local tag's
/// message. Its object is fetched alone under a probe ref, then the object
/// the probe ref named is compared through [`tags_identical`], never the
/// advertised one: the remote may have moved since the listing, and an
/// advertised object it no longer holds was never fetched. The probe ref is
/// removed whatever happened, so no ref outside the project's namespaces
/// survives. A tag gone from the remote is not identical; the push lease
/// then reports where it went.
fn probe_identical(repo: &Repository, publish: &Publish<'_>) -> Result<bool> {
    let fetched = crate::git::remote::probe_remote_tag(
        repo,
        publish.url,
        publish.tag,
        publish.token.clone(),
        Some(publish.deadline),
    )?;
    match fetched {
        Some(remote_object) => tags_identical(repo, publish.object, remote_object),
        None => Ok(false),
    }
}

/// Test seam: the remote moving between the listing and the probe or the
/// push is a race no test can win from outside, so a test registers a hook,
/// keyed by remote URL so parallel tests never see each other's, that runs
/// once right after the listing. Compiled out of production builds.
#[cfg(test)]
pub(crate) mod after_listing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    type Hook = Box<dyn FnOnce() + Send>;
    static HOOKS: Mutex<Option<HashMap<String, Hook>>> = Mutex::new(None);

    pub(crate) fn set(url: &str, hook: impl FnOnce() + Send + 'static) {
        HOOKS.lock().unwrap().get_or_insert_with(HashMap::new).insert(url.into(), Box::new(hook));
    }

    pub(super) fn fire(url: &str) {
        let hook = HOOKS.lock().unwrap().as_mut().and_then(|m| m.remove(url));
        if let Some(hook) = hook {
            hook();
        }
    }
}

/// `git.remote_push_tags` (SPEC-0054): refusals come in the order of DEC-005
/// because that is the order of the calls: membership (`open`), remote
/// lookup, credential, then inside [`publish_all_tags`] the connection, the
/// number and the push. A paused operation does not block it (FR-NEW-014):
/// publishing touches no working file and no index row.
pub(crate) async fn tool_remote_push_tags(
    ctx: ToolCtx,
    a: crate::mcp::server::GitRemotePushTagsArgs,
    g: Option<Arc<GitRepoStore>>,
    t: Option<Arc<crate::git::OAuthTokenStore>>,
) -> Result<Value> {
    let entry = open(&ctx, &a.mount_id, g).await?;
    let (remote, url) = tracked_remote(&entry, &a.remote).await?;
    let (token, auth) = resolve_clone_credential(&ctx, &t, &url).await?;
    let target = PublishAllTarget {
        mount_id: &a.mount_id,
        person: &ctx.person,
        remote: &remote,
        url: &url,
        token,
        auth: &auth,
        timeout_secs: ctx.state.config.git.remote_timeout_secs,
        confirm_count: a.confirm_count.as_ref(),
    };
    publish_all_tags(entry, ctx.state.safety.clone(), target).await
}

/// FR-NEW-010, DEC-017: the remote name to use (an empty text means
/// `origin`, since the serde default only covers an absent field) and its
/// URL. Names are compared exactly, so `Origin` is not `origin`.
pub(crate) async fn tracked_remote(entry: &GitRepoEntry, remote: &str) -> Result<(String, String)> {
    let remote = if remote.is_empty() { "origin" } else { remote };
    let remotes = entry.db.list_remotes().await?;
    match remotes.iter().find(|(n, _)| n == remote) {
        Some((_, url)) => Ok((remote.to_string(), url.clone())),
        None if remotes.is_empty() => {
            Err(ToolError::not_found("Cannot publish tags: this project tracks no remote."))
        }
        None => Err(ToolError::not_found(format!(
            "Cannot publish tags: this project does not track a remote named '{remote}'."
        ))),
    }
}

/// Where and how every due tag is published (SPEC-0054): everything
/// [`publish_all_tags`] needs once the remote URL and the credential are
/// known. `confirm_count` absent or null is a preview.
pub(crate) struct PublishAllTarget<'a> {
    pub mount_id: &'a str,
    pub person: &'a str,
    pub remote: &'a str,
    pub url: &'a str,
    pub token: Option<String>,
    pub auth: &'a str,
    pub timeout_secs: u64,
    pub confirm_count: Option<&'a Value>,
}

/// SPEC-0054 DEC-005 to DEC-012: preview or publish every local tag once the
/// remote and the credential are known. Split from [`tool_remote_push_tags`]
/// so a test can reach it with a `file://` remote, which the tool refuses.
///
/// The whole action holds the write lock (DEC-007), preview included, so the
/// count shown and the count checked come from one consistent view. One
/// listing connection decides every outcome, then at most one push sends
/// every due tag. One audit line per tag the push really published.
///
/// Cost: hydration, O(o) in stored objects as for every tag tool; then for t
/// local tags and n advertised refs, O(n) to index the listing, O(t log t)
/// to sort, and one history walk per distinct local commit not advertised
/// itself, O(u d) for u such commits and depth d, memoized so tags sharing a
/// commit cost one walk; one probe connection per annotated lookalike. O(n +
/// t) space.
pub(crate) async fn publish_all_tags(
    entry: Arc<GitRepoEntry>,
    safety: Arc<SafetyManager>,
    target: PublishAllTarget<'_>,
) -> Result<Value> {
    let host = crate::git::remote::extract_host(target.url);
    let (remote, auth) = (target.remote, target.auth);
    crate::git::remote::trace_remote_operation(
        "git.remote_push_tags",
        remote,
        &host,
        auth,
        publish_all_inner(entry, safety, target, &host),
    )
    .await
}

async fn publish_all_inner(
    entry: Arc<GitRepoEntry>,
    safety: Arc<SafetyManager>,
    target: PublishAllTarget<'_>,
    host: &str,
) -> Result<Value> {
    let PublishAllTarget {
        mount_id,
        person,
        remote,
        url,
        token,
        auth,
        timeout_secs,
        confirm_count,
    } = target;
    // Held in the async caller, never inside the blocking closure, so a
    // deadline firing releases it, as for a single publication.
    let _write = entry.write_lock.lock().await;
    // `String` ordering is byte ordering (DEC-006), never the database
    // collation; the full refs share one prefix, so this sorts the names.
    let mut local: Vec<(String, String)> = entry
        .db
        .list_refs()
        .await?
        .into_iter()
        .filter(|r| !r.symbolic && r.name.starts_with(TAGS_PREFIX))
        .map(|r| (r.name, r.target))
        .collect();
    local.sort_unstable();
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let (thread_entry, url_owned, confirm) =
        (entry.clone(), url.to_string(), confirm_count.cloned());
    let report = crate::git::remote::with_remote_deadline(
        timeout_secs,
        host,
        on_git_thread(move || async move {
            let repo = thread_entry.repo.lock().await;
            hydrate(&thread_entry, &repo).await?;
            let conn = Connection { url: &url_owned, token, deadline };
            publish_all_on_repo(&repo, &local, &conn, confirm.as_ref())
        }),
    )
    .await?;

    for tag in report.tags.iter().filter(|t| t.outcome == Outcome::Published) {
        safety.record_audit(
            person,
            &entry.project_id,
            "git.tag_publish",
            "/",
            &format!("published tag '{}' at {}", tag.name, tag.commit),
        );
    }
    let published = report.tags.iter().filter(|t| t.outcome == Outcome::Published).count();
    let message = match (report.sent, report.to_publish) {
        (true, _) => String::new(),
        (false, 0) => "No tag to publish.".to_string(),
        (false, n) => format!("Tags to publish: {n}. Confirm by stating {n} to send them."),
    };
    Ok(json!({
        "mount_id": mount_id,
        "remote": remote,
        "to_publish": report.to_publish,
        "published": published,
        "sent": report.sent,
        "tags": report.tags.iter().map(TagOutcome::to_json).collect::<Vec<_>>(),
        "lines": render(&report.tags),
        "message": message,
        "auth": auth,
    }))
}

/// The remote connection of one blocking bulk publication.
struct Connection<'a> {
    url: &'a str,
    token: Option<String>,
    deadline: Instant,
}

/// The outcome of one local tag (DEC-006, DEC-011).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    ToPublish,
    Published,
    UpToDate,
    /// The remote tag's commit, full id.
    Differs(String),
    CommitNotOnRemote,
    /// The reason the remote, or git itself, gave.
    Refused(String),
}

/// One local tag and what happened to it.
struct TagOutcome {
    name: String,
    object: Oid,
    /// The commit the tag stands for, full id.
    commit: String,
    outcome: Outcome,
}

impl TagOutcome {
    /// `<name> <outcome>`, the one line per tag every answer shares (DEC-009).
    fn line(&self) -> String {
        match &self.outcome {
            Outcome::ToPublish => format!("{} to publish", self.name),
            Outcome::Published => format!("{} published", self.name),
            Outcome::UpToDate => format!("{} up to date", self.name),
            Outcome::Differs(at) => format!("{} differs (remote at {})", self.name, short7(at)),
            Outcome::CommitNotOnRemote => format!("{} commit not on remote", self.name),
            Outcome::Refused(why) => format!("{} refused by remote: {why}", self.name),
        }
    }

    fn to_json(&self) -> Value {
        let outcome = match &self.outcome {
            Outcome::ToPublish => "to publish",
            Outcome::Published => "published",
            Outcome::UpToDate => "up to date",
            Outcome::Differs(_) => "differs",
            Outcome::CommitNotOnRemote => "commit not on remote",
            Outcome::Refused(_) => "refused by remote",
        };
        let mut out = json!({"name": self.name, "outcome": outcome, "commit": self.commit});
        if let Some(map) = out.as_object_mut() {
            match &self.outcome {
                Outcome::Differs(at) => {
                    map.insert("remote_commit".to_string(), Value::String(at.clone()));
                }
                Outcome::Refused(why) => {
                    map.insert("reason".to_string(), Value::String(why.clone()));
                }
                _ => {}
            }
        }
        out
    }
}

/// What the blocking half decided: every tag's outcome, the count of tags
/// to publish at the listing, and whether a push happened.
struct BulkReport {
    tags: Vec<TagOutcome>,
    to_publish: usize,
    sent: bool,
}

/// DEC-009: the per tag lines, in the order of `tags`, shared by the
/// preview, the report and the wrong number refusal so they cannot drift.
fn render(tags: &[TagOutcome]) -> Vec<String> {
    tags.iter().map(TagOutcome::line).collect()
}

/// The blocking half of a bulk publication, in the order of DEC-005: list,
/// classify, check the number, then push the due tags in one connection.
fn publish_all_on_repo(
    repo: &Repository,
    local: &[(String, String)],
    conn: &Connection<'_>,
    confirm: Option<&Value>,
) -> Result<BulkReport> {
    let advertised = crate::git::remote::list_advertised_refs(
        repo,
        conn.url,
        conn.token.clone(),
        Some(conn.deadline),
    )?;
    #[cfg(test)]
    after_listing::fire(conn.url);

    let mut tags = classify_tags(repo, local, &advertised, conn)?;
    let to_publish = tags.iter().filter(|t| t.outcome == Outcome::ToPublish).count();
    let Some(stated) = parse_confirm(confirm)? else {
        return Ok(BulkReport { tags, to_publish, sent: false });
    };
    if stated != to_publish as u64 {
        let mut message = format!(
            "Tags to publish: {to_publish}, not {stated}. Review the list, then confirm again."
        );
        for line in render(&tags) {
            message.push('\n');
            message.push_str(&line);
        }
        return Err(ToolError::no_clobber(message));
    }
    if to_publish == 0 {
        return Ok(BulkReport { tags, to_publish, sent: false });
    }
    send_due_tags(repo, &mut tags, conn)?;
    Ok(BulkReport { tags, to_publish, sent: true })
}

/// DEC-006: the outcome of every local tag against the one listing, in the
/// order of `local`. Checks, in order: commit not on remote, identical,
/// differs, to publish.
fn classify_tags(
    repo: &Repository,
    local: &[(String, String)],
    advertised: &[(String, Oid)],
    conn: &Connection<'_>,
) -> Result<Vec<TagOutcome>> {
    let by_name: std::collections::HashMap<&str, Oid> =
        advertised.iter().map(|(n, oid)| (n.as_str(), *oid)).collect();
    let tips: std::collections::HashSet<Oid> = advertised.iter().map(|(_, oid)| *oid).collect();
    let mut on_remote: std::collections::HashMap<Oid, bool> = std::collections::HashMap::new();
    let mut out = Vec::with_capacity(local.len());
    for (full_ref, target) in local {
        let name = full_ref.strip_prefix(TAGS_PREFIX).unwrap_or(full_ref).to_string();
        let object = parse_oid(target)?;
        // A target that names no commit the project holds cannot be on the
        // remote either; it is reported, not allowed to fail the whole list.
        let Some(commit) = peel_to_commit(repo, target) else {
            let commit = target.clone();
            out.push(TagOutcome { name, object, commit, outcome: Outcome::CommitNotOnRemote });
            continue;
        };
        let commit_oid = parse_oid(&commit)?;
        let reachable = *on_remote
            .entry(commit_oid)
            .or_insert_with(|| commit_reachable(repo, commit_oid, &tips));
        let outcome = if reachable {
            remote_outcome(repo, full_ref, &name, object, commit_oid, &by_name, conn)?
        } else {
            Outcome::CommitNotOnRemote
        };
        out.push(TagOutcome { name, object, commit, outcome });
    }
    Ok(out)
}

/// Identical, differs or to publish, for a tag whose commit is on the
/// remote, with the rules of the single publication (SPEC-0052 DEC-017).
fn remote_outcome(
    repo: &Repository,
    full_ref: &str,
    name: &str,
    object: Oid,
    commit: Oid,
    by_name: &std::collections::HashMap<&str, Oid>,
    conn: &Connection<'_>,
) -> Result<Outcome> {
    let Some(&remote_object) = by_name.get(full_ref) else {
        return Ok(Outcome::ToPublish);
    };
    let peeled = by_name.get(format!("{full_ref}^{{}}").as_str()).copied();
    let remote_commit = peeled.unwrap_or(remote_object);
    // Two annotated tags on one commit may carry one message under another
    // tagger or time, which only the remote's tag object can tell.
    let identical = remote_object == object
        || (peeled.is_some() && object != commit && remote_commit == commit && {
            let publish = Publish {
                tag: name,
                object,
                url: conn.url,
                token: conn.token.clone(),
                deadline: conn.deadline,
            };
            probe_identical(repo, &publish)?
        });
    Ok(if identical { Outcome::UpToDate } else { Outcome::Differs(remote_commit.to_string()) })
}

/// DEC-008: absent or null is a preview; a string of 1 to 9 ASCII digits or
/// a JSON integer from 0 to 999999999 is a number; anything else is refused
/// with one text, so a typo never reads as a transport error.
fn parse_confirm(confirm: Option<&Value>) -> Result<Option<u64>> {
    const MAX_DIGITS: usize = 9;
    const MAX: u64 = 999_999_999;
    let refused = || {
        ToolError::invalid_argument("The number to confirm must be a whole number, zero or more.")
    };
    match confirm {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s))
            if (1..=MAX_DIGITS).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) =>
        {
            s.parse().map(Some).map_err(|_| refused())
        }
        Some(Value::Number(n)) => n.as_u64().filter(|v| *v <= MAX).map(Some).ok_or_else(refused),
        Some(_) => Err(refused()),
    }
}

/// DEC-010, DEC-011, DEC-012: write each due tag's ref on disk, as the single
/// publication does since a push names refs, then push them all in one
/// connection and mark each `published` or `refused by remote`.
///
/// FR-NEW-020: a lost lease means the remote gained some of those tags after
/// the listing and nothing was sent. The remote is listed again, each lost
/// tag reclassified from that fresh listing and, unless it is still due,
/// dropped before the rest is pushed again. Every round that loses a lease
/// removes at least one tag, so the loop is bounded by the number of due
/// tags: O(t) rounds of one listing and one push, in practice one.
fn send_due_tags(repo: &Repository, tags: &mut [TagOutcome], conn: &Connection<'_>) -> Result<()> {
    let mut due = Vec::new();
    for (at, tag) in tags.iter_mut().enumerate() {
        if tag.outcome != Outcome::ToPublish {
            continue;
        }
        let tag_ref = format!("{TAGS_PREFIX}{}", tag.name);
        // Checked rather than left to `reference`, which quietly normalises
        // `a//b` to `a/b` and would push a ref of another name (DEC-019).
        if !git2::Reference::is_valid_name(&tag_ref) {
            tag.outcome = Outcome::Refused("invalid ref name".to_string());
            continue;
        }
        match repo.reference(&tag_ref, tag.object, true, "mcp-fs git.remote_push_tags") {
            Ok(_) => due.push(at),
            Err(e) => tag.outcome = Outcome::Refused(e.message().to_string()),
        }
    }
    for _ in 0..due.len() {
        if due.is_empty() {
            return Ok(());
        }
        let names: Vec<&str> = due.iter().map(|&at| tags[at].name.as_str()).collect();
        let pushed = crate::git::remote::push_tags_to_remote(
            repo,
            conn.url,
            &names,
            conn.token.clone(),
            Some(conn.deadline),
        )?;
        let rejected: std::collections::HashMap<String, String> =
            pushed.rejected.into_iter().collect();
        due.retain(|&at| match rejected.get(&tags[at].name) {
            Some(why) => {
                tags[at].outcome = Outcome::Refused(why.clone());
                false
            }
            None => true,
        });
        if pushed.lease_lost.is_empty() {
            for &at in &due {
                tags[at].outcome = Outcome::Published;
            }
            return Ok(());
        }
        let before = due.len();
        reclassify_lost(repo, tags, &mut due, &pushed.lease_lost, conn)?;
        if due.len() == before {
            break;
        }
    }
    if due.is_empty() {
        return Ok(());
    }
    // Only reachable when a fresh listing shows the lost tags due again: the
    // remote keeps changing them faster than a push can follow. Nothing of
    // the remaining tags was sent.
    let names: Vec<&str> = due.iter().map(|&at| tags[at].name.as_str()).collect();
    Err(ToolError::no_clobber(format!(
        "Remote tags kept changing while publishing: {}. Review the list, then confirm again.",
        names.join(", ")
    )))
}

/// FR-NEW-020, DEC-011: list the remote again and give each tag whose lease
/// was lost its outcome against that listing; a tag the remote now holds is
/// `up to date` or `differs` and leaves `due`, one it no longer holds stays.
fn reclassify_lost(
    repo: &Repository,
    tags: &mut [TagOutcome],
    due: &mut Vec<usize>,
    lost: &[String],
    conn: &Connection<'_>,
) -> Result<()> {
    let advertised = crate::git::remote::list_advertised_refs(
        repo,
        conn.url,
        conn.token.clone(),
        Some(conn.deadline),
    )?;
    let by_name: std::collections::HashMap<&str, Oid> =
        advertised.iter().map(|(n, oid)| (n.as_str(), *oid)).collect();
    let mut kept = Vec::with_capacity(due.len());
    for &at in due.iter() {
        let tag = &tags[at];
        if !lost.contains(&tag.name) {
            kept.push(at);
            continue;
        }
        let full_ref = format!("{TAGS_PREFIX}{}", tag.name);
        let commit = parse_oid(&tag.commit)?;
        let outcome =
            remote_outcome(repo, &full_ref, &tag.name, tag.object, commit, &by_name, conn)?;
        if outcome == Outcome::ToPublish {
            kept.push(at);
        } else {
            tags[at].outcome = outcome;
        }
    }
    *due = kept;
    Ok(())
}

/// `git.remote_delete_tag`: refusals come in the order of DEC-005 because
/// that is the order of the calls: membership (`open`), remote lookup,
/// credential, then inside [`delete_remote_tag`] the connection and the tag
/// checks. A paused operation does not block it (FR-NEW-010): the deletion
/// touches no local file, ref or index row.
pub(crate) async fn tool_remote_delete_tag(
    ctx: ToolCtx,
    a: crate::mcp::server::GitRemoteDeleteTagArgs,
    g: Option<Arc<GitRepoStore>>,
    t: Option<Arc<crate::git::OAuthTokenStore>>,
) -> Result<Value> {
    let entry = open(&ctx, &a.mount_id, g).await?;
    let remotes = entry.db.list_remotes().await?;
    let Some(url) = remotes.iter().find(|(n, _)| *n == a.remote).map(|(_, u)| u.clone()) else {
        return Err(ToolError::not_found(if remotes.is_empty() {
            format!("Cannot delete tag '{}': this project tracks no remote.", a.tag)
        } else {
            format!(
                "Cannot delete tag '{}': this project does not track a remote named '{}'.",
                a.tag, a.remote
            )
        }));
    };
    let (token, auth) = resolve_clone_credential(&ctx, &t, &url).await?;
    let target = DeleteTarget {
        mount_id: &a.mount_id,
        person: &ctx.person,
        remote: &a.remote,
        tag: &a.tag,
        url: &url,
        token,
        auth: &auth,
        timeout_secs: ctx.state.config.git.remote_timeout_secs,
        expected_remote_commit: &a.expected_remote_commit,
    };
    delete_remote_tag(entry, ctx.state.safety.clone(), target).await
}

/// Where and how one remote tag is deleted (SPEC-0053): everything
/// [`delete_remote_tag`] needs once the remote URL and the credential are
/// known.
pub(crate) struct DeleteTarget<'a> {
    pub mount_id: &'a str,
    pub person: &'a str,
    pub remote: &'a str,
    pub tag: &'a str,
    pub url: &'a str,
    pub token: Option<String>,
    pub auth: &'a str,
    pub timeout_secs: u64,
    /// The commit the requester states the remote tag points at now.
    pub expected_remote_commit: &'a str,
}

/// SPEC-0053 DEC-005, DEC-010: delete one remote tag once the remote and
/// the credential are known. Split from [`tool_remote_delete_tag`] so a test
/// can reach it with a `file://` remote, which the registered tool refuses.
///
/// Under the write lock, one listing connection decides every refusal, then
/// at most one delete push. The audit entry is written only once the push
/// returned Ok, so no refusal and no failure leaves a line (FR-NEW-008).
///
/// Cost: hydration, O(o) in stored objects as for every tag tool; then for n
/// advertised refs O(n) time and space for the listing and the lookups. The
/// delete push sends no object; a lease lost between listing and push costs
/// one more listing, O(n).
pub(crate) async fn delete_remote_tag(
    entry: Arc<GitRepoEntry>,
    safety: Arc<SafetyManager>,
    target: DeleteTarget<'_>,
) -> Result<Value> {
    let host = crate::git::remote::extract_host(target.url);
    let (remote, auth) = (target.remote, target.auth);
    crate::git::remote::trace_remote_operation(
        "git.remote_delete_tag",
        remote,
        &host,
        auth,
        delete_inner(entry, safety, target, &host),
    )
    .await
}

async fn delete_inner(
    entry: Arc<GitRepoEntry>,
    safety: Arc<SafetyManager>,
    target: DeleteTarget<'_>,
    host: &str,
) -> Result<Value> {
    let DeleteTarget {
        mount_id,
        person,
        remote,
        tag,
        url,
        token,
        auth,
        timeout_secs,
        expected_remote_commit,
    } = target;
    // Held in the async caller, never inside the blocking closure, so a
    // deadline firing releases it, as for a publication.
    let _write = entry.write_lock.lock().await;
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let (thread_entry, tag_owned, url_owned, stated) =
        (entry.clone(), tag.to_string(), url.to_string(), expected_remote_commit.to_string());
    let commit = crate::git::remote::with_remote_deadline(
        timeout_secs,
        host,
        on_git_thread(move || async move {
            let repo = thread_entry.repo.lock().await;
            hydrate(&thread_entry, &repo).await?;
            delete_on_repo(&repo, &tag_owned, &url_owned, token, deadline, &stated)
        }),
    )
    .await?;

    safety.record_audit(
        person,
        &entry.project_id,
        "git.tag_remote_delete",
        "/",
        &format!("deleted tag '{tag}' on remote '{remote}' (was at {commit})"),
    );
    Ok(json!({
        "mount_id": mount_id,
        "tag": tag,
        "remote": remote,
        "deleted_commit": commit,
        "auth": auth,
    }))
}

/// The blocking half of a deletion: list, check in the order of DEC-005,
/// then push the deletion. Returns the full commit the remote tag had.
fn delete_on_repo(
    repo: &Repository,
    tag: &str,
    url: &str,
    token: Option<String>,
    deadline: Instant,
    stated: &str,
) -> Result<String> {
    let tag_ref = format!("{TAGS_PREFIX}{tag}");
    let advertised =
        crate::git::remote::list_advertised_refs(repo, url, token.clone(), Some(deadline))?;
    #[cfg(test)]
    after_listing::fire(url);

    // DEC-007: only an advertised tag is ever named in a refspec. A name
    // ending in `^{}` would otherwise match the peeled entry of another tag.
    let Some((object, commit)) =
        remote_tag_state(&advertised, &tag_ref).filter(|_| !tag.ends_with("^{}"))
    else {
        return Err(remote_tag_absent(tag));
    };
    if stated.trim().is_empty() {
        return Err(ToolError::invalid_argument(format!(
            "Remote tag '{tag}' points at {}. State that commit to confirm its deletion.",
            short7(&commit)
        )));
    }
    if !stated_commit_matches(stated, &commit) {
        return Err(moved(tag, &commit));
    }

    let pushed = crate::git::remote::delete_tag_on_remote(
        repo,
        url,
        tag,
        token.clone(),
        Some(deadline),
        object,
    )?;
    if pushed == crate::git::remote::TagPush::LeaseLost {
        // DEC-008: the tag moved or vanished between the listing and the
        // push, and nothing was sent. One more listing says which; the push
        // is never retried, since the member confirmed only the old commit.
        let now =
            crate::git::remote::list_advertised_refs(repo, url, token.clone(), Some(deadline))?;
        return Err(match remote_tag_state(&now, &tag_ref) {
            Some((_, now_commit)) => moved(tag, &now_commit),
            None => remote_tag_absent(tag),
        });
    }
    Ok(commit)
}

/// The remote tag's own object and the commit it stands for (its peeled
/// `^{}` entry for an annotated tag), from one listing; `None` when absent.
fn remote_tag_state(advertised: &[(String, Oid)], tag_ref: &str) -> Option<(Oid, String)> {
    let object = advertised_oid(advertised, tag_ref)?;
    let commit = advertised_oid(advertised, &format!("{tag_ref}^{{}}")).unwrap_or(object);
    Some((object, commit.to_string()))
}

fn remote_tag_absent(tag: &str) -> ToolError {
    ToolError::not_found(format!("Remote tag '{tag}' does not exist."))
}

fn moved(tag: &str, remote_commit: &str) -> ToolError {
    ToolError::no_clobber(format!(
        "Remote tag '{tag}' has moved to {} since you last saw it. Fetch, then confirm again.",
        short7(remote_commit)
    ))
}

/// DEC-020: the commit is on the remote when an advertised ref names it, or
/// when it is an ancestor of an advertised commit the project already holds.
/// A tip the project lacks is skipped rather than fetched, so such a commit
/// counts as absent even when the remote has it (E2E-084).
fn commit_on_remote(repo: &Repository, commit: Oid, advertised: &[(String, Oid)]) -> bool {
    let tips: std::collections::HashSet<Oid> = advertised.iter().map(|(_, oid)| *oid).collect();
    commit_reachable(repo, commit, &tips)
}

/// [`commit_on_remote`] against a set of tips built once, so a caller
/// checking many commits against one listing does not rebuild it.
fn commit_reachable(repo: &Repository, commit: Oid, tips: &std::collections::HashSet<Oid>) -> bool {
    tips.contains(&commit)
        || tips.iter().any(|&tip| {
            repo.find_commit(tip).is_ok() && repo.graph_descendant_of(tip, commit).unwrap_or(false)
        })
}

fn advertised_oid(advertised: &[(String, Oid)], name: &str) -> Option<Oid> {
    advertised.iter().find(|(n, _)| n == name).map(|(_, oid)| *oid)
}

/// DEC-018: a stated commit names `commit` as its full id or as a prefix of
/// at least 7 hex characters; anything shorter is too ambiguous to lease on.
pub(crate) fn stated_commit_matches(stated: &str, commit: &str) -> bool {
    let stated = stated.trim().to_ascii_lowercase();
    stated.len() >= 7
        && stated.chars().all(|c| c.is_ascii_hexdigit())
        && commit.starts_with(&stated)
}

fn unknown_tag(name: &str) -> ToolError {
    ToolError::not_found(format!("Tag '{name}' does not exist."))
}

/// DEC-018: tag refusals show 7 characters, distinct from the 8 character
/// `short` the other git tools print.
pub(crate) fn short7(sha: &str) -> String {
    sha.chars().take(7).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short7_keeps_seven_characters() {
        assert_eq!(short7("a1b2c3d4e5f6"), "a1b2c3d");
        assert_eq!(short7("abc"), "abc");
    }

    /// DEC-019: a per ref refusal from a real remote, the path the local
    /// file transport never takes, reads the same as git's own name refusal.
    #[test]
    fn a_remote_name_refusal_names_the_tag_and_the_reason() {
        let err = crate::git::remote::remote_refused_tag("a//b", "invalid ref name");
        assert_eq!(err.code, crate::errors::code::INTERNAL_ERROR);
        assert_eq!(err.message, "Remote refused tag 'a//b': invalid ref name");
    }
}
