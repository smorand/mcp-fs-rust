//! Tag engines (SPEC-0052): `git.tag_create`, `git.tag_delete`, the
//! `git.tags` listing and `git.remote_push_tag`.
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
/// O(n + u d) time for u distinct tips and O(n) space. Nothing is fetched.
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
    let PublishTarget { mount_id, person, remote, tag, url, token, auth, timeout_secs } = target;
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
    let (commit, pushed) = crate::git::remote::with_remote_deadline(
        timeout_secs,
        host,
        on_git_thread(move || async move {
            let repo = thread_entry.repo.lock().await;
            hydrate(&thread_entry, &repo).await?;
            publish_on_repo(&repo, &tag_owned, object, &url_owned, token, deadline)
        }),
    )
    .await?;

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
        "created": pushed,
        "up_to_date": !pushed,
        "remote_sha": commit,
        "auth": auth,
    }))
}

/// The blocking half of a publication: list, check, then push only a tag
/// the remote lacks. Returns the tag's commit and whether a push happened.
fn publish_on_repo(
    repo: &Repository,
    tag: &str,
    object: Oid,
    url: &str,
    token: Option<String>,
    deadline: Instant,
) -> Result<(String, bool)> {
    let tag_ref = format!("{TAGS_PREFIX}{tag}");
    let commit = peel_to_commit(repo, &object.to_string()).ok_or_else(|| {
        ToolError::internal(format!("tag '{tag}' does not name a commit held by the project"))
    })?;
    let commit_oid = parse_oid(&commit)?;
    let advertised =
        crate::git::remote::list_advertised_refs(repo, url, token.clone(), Some(deadline))?;

    if !commit_on_remote(repo, commit_oid, &advertised) {
        return Err(ToolError::invalid_argument(format!(
            "Cannot publish tag '{tag}': commit {} is not on the remote. Publish its branch first.",
            short7(&commit)
        )));
    }

    if let Some(remote_object) = advertised_oid(&advertised, &tag_ref) {
        // The same object is the same tag, whatever its kind. A different
        // object is refused for now: telling an identical annotated tag
        // written twice apart from a real difference needs the message
        // probe of DEC-017, and overwriting needs the confirmation of
        // FR-NEW-008, both outside this tool's first version.
        if remote_object == object {
            return Ok((commit, false));
        }
        let peeled =
            advertised_oid(&advertised, &format!("{tag_ref}^{{}}")).unwrap_or(remote_object);
        return Err(ToolError::no_clobber(format!(
            "Remote tag '{tag}' already exists at a different commit ({}). Confirm overwriting \
             to replace it.",
            short7(&peeled.to_string())
        )));
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
    crate::git::remote::push_tag_to_remote(repo, url, tag, token, Some(deadline))?;
    Ok((commit, true))
}

/// DEC-020: the commit is on the remote when an advertised ref names it, or
/// when it is an ancestor of an advertised commit the project already holds.
/// A tip the project lacks is skipped rather than fetched, so such a commit
/// counts as absent even when the remote has it (E2E-084).
fn commit_on_remote(repo: &Repository, commit: Oid, advertised: &[(String, Oid)]) -> bool {
    let tips: std::collections::HashSet<Oid> = advertised.iter().map(|(_, oid)| *oid).collect();
    tips.contains(&commit)
        || tips.into_iter().any(|tip| {
            repo.find_commit(tip).is_ok() && repo.graph_descendant_of(tip, commit).unwrap_or(false)
        })
}

fn advertised_oid(advertised: &[(String, Oid)], name: &str) -> Option<Oid> {
    advertised.iter().find(|(n, _)| n == name).map(|(_, oid)| *oid)
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
