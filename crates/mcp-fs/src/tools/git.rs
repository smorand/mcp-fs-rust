//! `git.*` tools: init, status, branches, tags, log, show, diff, commit,
//! checkout_file, blame, remote_clone.
//!
//! Port of the C# `Tools/GitTools.cs`. Registered only when `git.enabled`.
//!
//! Every tool is gated by project membership (`state.authorize`), never by the
//! platform admin role: administering the platform is not the same as reading a
//! project's source history.
//!
//! Two structural differences from the C#, both forced by `git2` not exposing a
//! custom libgit2 ODB backend (see [`crate::git::odb`] for the full reasoning):
//!
//! * before any libgit2 read (log, show, diff, blame, checkout_file) the blob
//!   backed object store is exported into the bare repo's on disk ODB, and after
//!   any libgit2 write (commit, remote_clone) the new objects are imported back.
//! * `git.remote_clone` imports the temporary clone's objects directly through
//!   [`crate::git::odb::BlobObjectDb::import_from_repo`] instead of the C# trick
//!   of adding the temp dir as a `file://` remote and fetching from it.
//!
//! `git2::Repository` is `Send` but not `Sync`, so a future holding a reference to
//! one is not `Send` and cannot be awaited by the MCP dispatcher. Every libgit2
//! touching section therefore runs through [`on_git_thread`], exactly like the git
//! HTTP handlers do.

use crate::errors::{Result, ToolError};
use crate::git::db::SqliteGitDb;
use crate::git::{GitRepoEntry, GitRepoStore};
use crate::mcp::registry::{ToolCtx, handler};
use crate::mcp::{ToolRegistry, ToolSchema};
use crate::storage::VolumeClient;
use chrono::{DateTime, FixedOffset, Utc};
use git2::{DiffFormat, DiffOptions, Oid, Repository, Tree};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// Mode bits used when a commit tree is built from the volume: every file is a
/// non executable regular file, exactly like the C# `Mode.NonExecutableFile`.
const MODE_FILE: i32 = 0o100_644;
const MODE_DIR: i32 = 0o040_000;

/// Diff context lines, matching the C# `CompareOptions { ContextLines = 3 }`.
const DIFF_CONTEXT_LINES: u32 = 3;

/// Register the eleven `git.*` tools (the three `git.auth*` ones live in
/// [`super::git_auth`]).
pub fn register(reg: &mut ToolRegistry) {
    register_with(reg, None, None);
}

/// Registration with injected dependencies, for tests. `None` falls back to the
/// process wide [`GitRepoStore`] and OAuth token store, which is what the server
/// wants: the tools and the git HTTP routes must share repository handles and
/// write locks, and `git.auth` must write the store `git.remote_clone` reads.
pub fn register_with(
    reg: &mut ToolRegistry,
    git: Option<Arc<GitRepoStore>>,
    tokens: Option<Arc<crate::git::OAuthTokenStore>>,
) {
    let g = git.clone();
    reg.add(
        ToolSchema::new("git.init", "Initialize the volume as a git repository.")
            .req_str("mount_id", "Project/volume id the operation targets."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let store = authorize(&ctx, &mount_id, g).await?;
                store.init_repo(&mount_id).await?;
                Ok(json!({
                    "mount_id": mount_id,
                    "initialized": true,
                    "message": "Git repository initialized",
                }))
            }
        }),
    );

    let g = git.clone();
    reg.add(
        ToolSchema::new("git.status", "Show HEAD, current branch, and all refs.")
            .req_str("mount_id", "Project/volume id the operation targets."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let entry = open(&ctx, &mount_id, g).await?;
                status(&mount_id, &entry).await
            }
        }),
    );

    let g = git.clone();
    reg.add(
        ToolSchema::new("git.branches", "List all branches with their SHA.")
            .req_str("mount_id", "Project/volume id the operation targets."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let entry = open(&ctx, &mount_id, g).await?;
                let branches = refs_under(&entry, "refs/heads/").await?;
                Ok(json!({"mount_id": mount_id, "branches": branches}))
            }
        }),
    );

    let g = git.clone();
    reg.add(
        ToolSchema::new("git.tags", "List all tags.")
            .req_str("mount_id", "Project/volume id the operation targets."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let entry = open(&ctx, &mount_id, g).await?;
                let tags = refs_under(&entry, "refs/tags/").await?;
                Ok(json!({"mount_id": mount_id, "tags": tags}))
            }
        }),
    );

    let g = git.clone();
    reg.add(
        ToolSchema::new("git.log", "List commits. ref_name defaults to HEAD.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .opt_str_null("ref_name", "Ref, branch, tag, or commit to start from; defaults to HEAD.")
            .opt_int("limit", 20, "Maximum number of commits to return.")
            .opt_str_null("path", "Optional path filter; only commits touching it are returned."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let ref_name = a.opt_str("ref_name");
                let limit = a.int_or("limit", 20);
                let path = a.opt_str("path");
                let entry = open(&ctx, &mount_id, g).await?;
                on_git_thread(move || async move {
                    log(&mount_id, &entry, ref_name.as_deref(), limit, path.as_deref()).await
                })
                .await
            }
        }),
    );

    let g = git.clone();
    reg.add(
        ToolSchema::new("git.show", "Show details and diff of a commit.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("commit_sha", "Commit SHA to show details and diff for."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let commit_sha = a.str("commit_sha")?;
                let entry = open(&ctx, &mount_id, g).await?;
                on_git_thread(move || async move { show(&entry, &commit_sha).await }).await
            }
        }),
    );

    let g = git.clone();
    reg.add(
        ToolSchema::new("git.diff", "Show diff between two refs or a ref and working tree.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("from_ref", "Base ref or commit to diff from.")
            .opt_str_null(
                "to_ref",
                "Target ref or commit to diff to; omit to diff against the working tree.",
            )
            .opt_str_null("path", "Optional path filter limiting the diff."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let from_ref = a.str("from_ref")?;
                let to_ref = a.opt_str("to_ref");
                let path = a.opt_str("path");
                let entry = open(&ctx, &mount_id, g).await?;
                on_git_thread(move || async move {
                    diff(&mount_id, &entry, &from_ref, to_ref.as_deref(), path.as_deref()).await
                })
                .await
            }
        }),
    );

    let g = git.clone();
    reg.add(
        ToolSchema::new("git.commit", "Create a commit from the current state of the volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("message", "Commit message.")
            .opt_str_null("author_name", "Optional author name; defaults to the caller.")
            .opt_str_null("author_email", "Optional author email; defaults to the caller person id."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let message = a.str("message")?;
                let author_name = a.opt_str("author_name");
                let author_email = a.opt_str("author_email");
                let entry = open(&ctx, &mount_id, g).await?;
                let client = ctx.state.stores.client(&mount_id).await?;
                let person = ctx.person.clone();
                on_git_thread(move || async move {
                    commit(&entry, &client, &person, &message, author_name, author_email).await
                })
                .await
            }
        }),
    );

    let g = git.clone();
    reg.add(
        ToolSchema::new("git.checkout_file", "Restore a file from a commit into the volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("commit_sha", "Commit SHA to restore the file from.")
            .req_str("path", "Absolute POSIX path of the file to restore into the volume."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let commit_sha = a.str("commit_sha")?;
                let path = a.str("path")?;
                let entry = open(&ctx, &mount_id, g).await?;
                let norm = ctx.state.safety.normalize_path(&path)?;
                let client = ctx.state.stores.client(&mount_id).await?;
                let bytes = {
                    let (entry, norm, commit_sha) = (entry, norm.clone(), commit_sha.clone());
                    on_git_thread(move || async move {
                        read_from_commit(&entry, &commit_sha, &norm).await
                    })
                    .await?
                };
                // A restore is a write, so it is charged like any other.
                ctx.state.safety.charge_write(&ctx.person, &mount_id, bytes.len() as i64)?;
                client.write_bytes_atomic(&norm, &bytes).await?;
                ctx.state.safety.record_audit(
                    &ctx.person,
                    &mount_id,
                    "git.checkout_file",
                    &norm,
                    &format!("from {commit_sha}"),
                );
                Ok(json!({"path": norm, "commit": commit_sha, "size": bytes.len()}))
            }
        }),
    );

    let g = git.clone();
    reg.add(
        ToolSchema::new("git.blame", "Show who last modified each line of a file.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Absolute POSIX path of the file to blame.")
            .opt_str_null("ref_name", "Ref or commit to blame from; defaults to HEAD."),
        handler(move |ctx: ToolCtx, a| {
            let g = g.clone();
            async move {
                let mount_id = a.str("mount_id")?;
                let path = a.str("path")?;
                let ref_name = a.opt_str("ref_name");
                let entry = open(&ctx, &mount_id, g).await?;
                let norm = ctx.state.safety.normalize_path(&path)?;
                on_git_thread(move || async move {
                    blame(&entry, &norm, ref_name.as_deref()).await
                })
                .await
            }
        }),
    );

    let g = git;
    reg.add(
        ToolSchema::new(
            "git.remote_clone",
            "Clone a remote git repository (GitHub, GitLab, or any HTTPS URL) into a volume. \
             Uses the OAuth token stored by git.auth if available for the detected provider. \
             Copies all files into the volume AND imports the full git history into the git backend. \
             Use depth=1 for a shallow clone (faster on large repos).",
        )
        .req_str("mount_id", "Project/volume id the clone is imported into.")
        .req_str("url", "Remote git repository URL (GitHub, GitLab, or any HTTPS URL).")
        .opt_str_null("branch", "Branch to clone; omit to use the remote default branch.")
        .opt_int("depth", 0, "Shallow clone depth; 0 clones the full history."),
        handler(move |ctx: ToolCtx, a| {
            let (g, t) = (g.clone(), tokens.clone());
            async move {
                let mount_id = a.str("mount_id")?;
                let url = a.str("url")?;
                let branch = a.opt_str("branch");
                let depth = a.int_or("depth", 0);
                let store = authorize(&ctx, &mount_id, g).await?;
                remote_clone(&ctx, store, t, &mount_id, &url, branch, depth).await
            }
        }),
    );
}

// ── gates and plumbing ──────────────────────────────────────────────────────

/// Membership gate, then the store to work with. Deliberately no platform admin
/// bypass: `state.authorize` is membership only.
async fn authorize(
    ctx: &ToolCtx,
    mount_id: &str,
    injected: Option<Arc<GitRepoStore>>,
) -> Result<Arc<GitRepoStore>> {
    ctx.state.authorize(mount_id, &ctx.person).await?;
    Ok(injected.unwrap_or_else(|| GitRepoStore::shared(ctx.state.config.clone())))
}

/// Authorize, require `git.init` to have run, and open the repository.
async fn open(
    ctx: &ToolCtx,
    mount_id: &str,
    injected: Option<Arc<GitRepoStore>>,
) -> Result<Arc<GitRepoEntry>> {
    let store = authorize(ctx, mount_id, injected).await?;
    if !store.is_initialized(mount_id).await {
        return Err(ToolError::not_found(format!(
            "git not initialized for mount '{mount_id}' (call git.init first)"
        )));
    }
    // Idempotent, and it also revives the entry after a server restart.
    store.get_or_open_repo(mount_id).await
}

/// Run a libgit2 touching future on the blocking pool.
///
/// `git2::Repository` is `Send` but not `Sync`, so a future holding a reference to
/// one is not `Send`; the MCP dispatcher needs `Send` futures. Moving the work to
/// the blocking pool also keeps CPU bound diff and pack work off the async
/// workers. Same helper as the git HTTP layer (private there).
async fn on_git_thread<T, F, Fut>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T>>,
{
    tokio::task::spawn_blocking(move || tokio::runtime::Handle::current().block_on(f()))
        .await
        .map_err(|e| ToolError::internal(format!("git task join: {e}")))?
}

fn git_err(what: &str, e: git2::Error) -> ToolError {
    ToolError::internal(format!("{what}: {e}"))
}

/// First 8 characters, the short sha the C# prints.
fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
}

fn parse_oid(sha: &str) -> Result<Oid> {
    Oid::from_str(sha).map_err(|_| ToolError::not_found(format!("commit '{sha}' not found")))
}

/// Resolve a ref name, a branch, a tag or a sha to a commit sha.
///
/// The C# order is reproduced including its quirk: a name made only of hex
/// characters is treated as a sha *before* `refs/heads/{name}` is tried, so a
/// branch named `beef` resolves to the sha `beef`. Kept for parity.
async fn resolve_ref(db: &SqliteGitDb, ref_or_sha: &str) -> Result<Option<String>> {
    if let Some(entry) = db.get_ref(ref_or_sha).await? {
        if entry.symbolic {
            return Ok(db.get_ref(&entry.target).await?.map(|r| r.target));
        }
        return Ok(Some(entry.target));
    }
    if ref_or_sha.len() >= 40 || ref_or_sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(Some(ref_or_sha.to_string()));
    }
    for prefix in ["refs/heads/", "refs/tags/"] {
        if let Some(r) = db.get_ref(&format!("{prefix}{ref_or_sha}")).await? {
            return Ok(Some(r.target));
        }
    }
    Ok(None)
}

/// Make libgit2 able to see the objects held in the blob store.
async fn hydrate(entry: &GitRepoEntry, repo: &Repository) -> Result<()> {
    entry.objects.export_to_repo(repo).await?;
    Ok(())
}

// ── tools ───────────────────────────────────────────────────────────────────

async fn status(mount_id: &str, entry: &GitRepoEntry) -> Result<Value> {
    let refs = entry.db.list_refs().await?;
    let head = entry.db.get_ref("HEAD").await?;
    let mut branch: Option<String> = None;
    let mut head_sha: Option<String> = None;
    match head {
        Some(h) if h.symbolic => {
            branch = Some(h.target.strip_prefix("refs/heads/").unwrap_or(&h.target).to_string());
            head_sha = entry.db.get_ref(&h.target).await?.map(|r| r.target);
        }
        Some(h) => head_sha = Some(h.target),
        None => {}
    }
    let listed: Vec<Value> = refs
        .iter()
        .filter(|r| !r.symbolic)
        .map(|r| json!({"name": r.name, "sha": r.target}))
        .collect();
    Ok(json!({
        "mount_id": mount_id,
        "head": head_sha,
        "branch": branch,
        "refs": listed,
    }))
}

/// `refs/heads/` for branches, `refs/tags/` for tags: same shape, same order.
async fn refs_under(entry: &GitRepoEntry, prefix: &'static str) -> Result<Vec<Value>> {
    Ok(entry
        .db
        .list_refs()
        .await?
        .into_iter()
        .filter(|r| !r.symbolic && r.name.starts_with(prefix))
        .map(|r| {
            json!({
                "name": r.name.strip_prefix(prefix).unwrap_or(&r.name),
                "full_ref": r.name,
                "sha": r.target,
            })
        })
        .collect())
}

async fn log(
    mount_id: &str,
    entry: &GitRepoEntry,
    ref_name: Option<&str>,
    limit: i64,
    path: Option<&str>,
) -> Result<Value> {
    let repo = entry.repo.lock().await;
    hydrate(entry, &repo).await?;

    let wanted = ref_name.unwrap_or("HEAD");
    let Some(start) = resolve_ref(&entry.db, wanted).await? else {
        // An empty repository has no HEAD target yet: an empty list, not an error.
        return Ok(json!({"mount_id": mount_id, "commits": []}));
    };

    let oid = parse_oid(&start)?;
    let commit = repo.find_commit(oid).map_err(|_| {
        // The ref points at a commit the object store does not have. This happens
        // when a volume was filled by a file copy that skipped git objects.
        ToolError::not_found(format!(
            "commit '{}' referenced by '{wanted}' is not present in the git object store. \
             Re-run git.remote_clone to import the full history.",
            short(&start)
        ))
    })?;

    let mut commits: Vec<Value> = Vec::new();
    let mut current = Some(commit);
    while let Some(c) = current.take() {
        if commits.len() as i64 >= limit {
            break;
        }
        let keep = match path {
            None => true,
            Some(p) => commit_touches_path(&repo, &c, p)?,
        };
        if keep {
            commits.push(commit_json(&c));
        }
        current = c.parents().next();
    }
    Ok(json!({"mount_id": mount_id, "commits": commits}))
}

async fn show(entry: &GitRepoEntry, commit_sha: &str) -> Result<Value> {
    let repo = entry.repo.lock().await;
    hydrate(entry, &repo).await?;
    let commit = repo
        .find_commit(parse_oid(commit_sha)?)
        .map_err(|_| ToolError::not_found(format!("commit '{commit_sha}' not found")))?;
    let tree = commit.tree().map_err(|e| git_err("commit tree", e))?;
    let parent_tree = match commit.parents().next() {
        Some(p) => Some(p.tree().map_err(|e| git_err("parent tree", e))?),
        None => None,
    };
    let text = generate_diff(&repo, parent_tree.as_ref(), Some(&tree), None)?;
    Ok(json!({"commit": commit_json(&commit), "diff": text}))
}

async fn diff(
    mount_id: &str,
    entry: &GitRepoEntry,
    from_ref: &str,
    to_ref: Option<&str>,
    path: Option<&str>,
) -> Result<Value> {
    let repo = entry.repo.lock().await;
    hydrate(entry, &repo).await?;

    let from_sha = resolve_ref(&entry.db, from_ref)
        .await?
        .ok_or_else(|| ToolError::not_found(format!("ref '{from_ref}' not found")))?;
    let from_commit = repo
        .find_commit(parse_oid(&from_sha)?)
        .map_err(|_| ToolError::not_found(format!("commit '{from_sha}' not found")))?;
    let from_tree = from_commit.tree().map_err(|e| git_err("commit tree", e))?;

    let to_tree = match to_ref {
        None => None,
        Some(r) => {
            let sha = resolve_ref(&entry.db, r)
                .await?
                .ok_or_else(|| ToolError::not_found(format!("ref '{r}' not found")))?;
            let c = repo
                .find_commit(parse_oid(&sha)?)
                .map_err(|_| ToolError::not_found(format!("commit '{sha}' not found")))?;
            Some(c.tree().map_err(|e| git_err("commit tree", e))?)
        }
    };

    let text = generate_diff(&repo, Some(&from_tree), to_tree.as_ref(), path)?;
    Ok(json!({"mount_id": mount_id, "from": from_ref, "to": to_ref, "diff": text}))
}

async fn commit(
    entry: &GitRepoEntry,
    client: &VolumeClient,
    person: &str,
    message: &str,
    author_name: Option<String>,
    author_email: Option<String>,
) -> Result<Value> {
    // Held for the whole commit so two callers cannot race the ref update.
    let _write = entry.write_lock.lock().await;
    let repo = entry.repo.lock().await;
    hydrate(entry, &repo).await?;

    let tree_oid = build_tree_from_volume(&repo, client).await?;
    let tree = repo.find_tree(tree_oid).map_err(|e| git_err("find tree", e))?;

    let name = author_name.unwrap_or_else(|| {
        person.split('@').next().unwrap_or(person).to_string()
    });
    let email = author_email.unwrap_or_else(|| person.to_string());
    let now = Utc::now().timestamp();
    let sig = git2::Signature::new(&name, &email, &git2::Time::new(now, 0))
        .map_err(|e| git_err("signature", e))?;

    let head = entry.db.get_ref("HEAD").await?;
    let branch_ref = match &head {
        Some(h) if h.symbolic => h.target.clone(),
        _ => "refs/heads/main".to_string(),
    };
    let parent_sha = match &head {
        Some(h) if h.symbolic => entry.db.get_ref(&h.target).await?.map(|r| r.target),
        Some(h) => Some(h.target.clone()),
        None => None,
    };
    let parent = match parent_sha {
        Some(sha) => Oid::from_str(&sha).ok().and_then(|o| repo.find_commit(o).ok()),
        None => None,
    };
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();

    let pretty = git2::message_prettify(message, None).map_err(|e| git_err("message", e))?;
    let oid = repo
        .commit(None, &sig, &sig, &pretty, &tree, &parents)
        .map_err(|e| git_err("create commit", e))?;
    let sha = oid.to_string();

    // Persist everything libgit2 just wrote (blobs, trees, the commit) into the
    // blob store, which is the authoritative object store. Without this the commit
    // would live only in the rebuildable on disk cache. This also indexes the
    // objects with their real size, where the C# recorded the commit with size 0.
    entry.objects.import_from_repo(&repo).await?;

    entry.db.set_ref(&branch_ref, &sha, false).await?;
    if head.is_none() {
        entry.db.set_ref("HEAD", &branch_ref, true).await?;
    }
    // Keep the on disk refs in step, so libgit2 based reads (blame, revwalk) and
    // an operator running `git log` inside the bare repo agree with the index.
    let _ = repo.reference(&branch_ref, oid, true, "mcp-fs git.commit");
    if head.is_none() {
        let _ = repo.set_head(&branch_ref);
    }

    Ok(json!({
        "commit_sha": sha,
        "message": message,
        "author": name,
        "timestamp": now,
    }))
}

/// The bytes of one path inside one commit.
async fn read_from_commit(entry: &GitRepoEntry, commit_sha: &str, norm: &str) -> Result<Vec<u8>> {
    let repo = entry.repo.lock().await;
    hydrate(entry, &repo).await?;
    let commit = repo
        .find_commit(parse_oid(commit_sha)?)
        .map_err(|_| ToolError::not_found(format!("commit '{commit_sha}' not found")))?;
    let tree = commit.tree().map_err(|e| git_err("commit tree", e))?;
    let rel = norm.trim_start_matches('/');
    let te = tree
        .get_path(Path::new(rel))
        .map_err(|_| ToolError::not_found(format!("'{norm}' not found in commit '{commit_sha}'")))?;
    let object = te.to_object(&repo).map_err(|e| git_err("read object", e))?;
    let blob = object
        .as_blob()
        .ok_or_else(|| ToolError::invalid_argument(format!("'{norm}' is not a file in that commit")))?;
    Ok(blob.content().to_vec())
}

async fn blame(entry: &GitRepoEntry, norm: &str, ref_name: Option<&str>) -> Result<Value> {
    let repo = entry.repo.lock().await;
    hydrate(entry, &repo).await?;
    let rel = norm.trim_start_matches('/');

    let wanted = ref_name.unwrap_or("HEAD");
    let Some(start) = resolve_ref(&entry.db, wanted).await? else {
        return Ok(json!({"path": norm, "lines": []}));
    };
    // An unknown start commit is an empty blame, not an error (C# parity).
    let Some(start_commit) = Oid::from_str(&start).ok().and_then(|o| repo.find_commit(o).ok())
    else {
        return Ok(json!({"path": norm, "lines": []}));
    };

    let mut opts = git2::BlameOptions::new();
    opts.newest_commit(start_commit.id());
    let blame = repo.blame_file(Path::new(rel), Some(&mut opts)).map_err(|e| {
        if e.code() == git2::ErrorCode::NotFound {
            ToolError::not_found(format!("'{norm}' not found in repository history"))
        } else {
            git_err("blame", e)
        }
    })?;

    let mut lines = Vec::new();
    for hunk in blame.iter() {
        let sig = hunk.final_signature();
        let author = sig.name().unwrap_or_default().to_string();
        let email = sig.email().unwrap_or_default().to_string();
        let date = format_git_time(sig.when(), "%Y-%m-%d");
        let sha = short(&hunk.final_commit_id().to_string());
        let first = hunk.final_start_line();
        for n in first..first + hunk.lines_in_hunk() {
            lines.push(json!({
                "line": n,
                "commit": sha,
                "author": author,
                "email": email,
                "date": date,
            }));
        }
    }
    Ok(json!({"path": norm, "lines": lines}))
}

#[allow(clippy::too_many_arguments)]
async fn remote_clone(
    ctx: &ToolCtx,
    store: Arc<GitRepoStore>,
    tokens: Option<Arc<crate::git::OAuthTokenStore>>,
    mount_id: &str,
    url: &str,
    branch: Option<String>,
    depth: i64,
) -> Result<Value> {
    // Which provider's token to look for, from the URL alone.
    let lower = url.to_ascii_lowercase();
    let provider = if lower.contains("github.com") {
        Some("github")
    } else if lower.contains("gitlab") {
        Some("gitlab")
    } else {
        None
    };

    let token = match provider {
        None => None,
        Some(p) => {
            let store = match tokens {
                Some(t) => t,
                None => super::git_auth::token_store(&ctx.state.config)?,
            };
            store.get_token(&ctx.person, p).map(|s| s.access_token)
        }
    };
    let auth = match (&token, provider) {
        (Some(_), Some(p)) => p.to_string(),
        _ => "anonymous".to_string(),
    };

    // The LLM sometimes sends the literal "null" or "HEAD", or an empty string.
    // All of those mean "whatever the remote's HEAD points at".
    let branch = branch.filter(|b| {
        let t = b.trim();
        !t.is_empty() && !t.eq_ignore_ascii_case("null") && !t.eq_ignore_ascii_case("head")
    });

    let tmp = TempClone::new()?;
    let tmp_path = tmp.path().to_path_buf();
    let (url_owned, branch_owned, mount_owned) =
        (url.to_string(), branch.clone(), mount_id.to_string());
    let person_owned = ctx.person.clone();
    let state = ctx.state.clone();

    on_git_thread(move || async move {
        let cloned = clone_to_temp(&url_owned, &tmp_path, branch_owned.as_deref(), depth, token)?;

        let head = cloned.head().ok();
        let target_branch = head
            .as_ref()
            .and_then(|h| h.shorthand())
            .filter(|n| !n.is_empty() && *n != "(no branch)")
            .map(str::to_string)
            .or_else(|| branch_owned.clone())
            .unwrap_or_else(|| "main".to_string());

        let tip = match head.as_ref().and_then(|h| h.peel_to_commit().ok()) {
            Some(c) => c,
            None => {
                return Ok(json!({
                    "mount_id": mount_owned,
                    "url": url_owned,
                    "files_imported": 0,
                    "message": "Repository is empty",
                }));
            }
        };

        if !store.is_initialized(&mount_owned).await {
            store.init_repo(&mount_owned).await?;
        }
        let entry = store.get_or_open_repo(&mount_owned).await?;

        // Import every object of the clone (packed included) into the blob store.
        // The C# added the temp dir as a file:// remote and fetched from it so its
        // custom ODB backend would see the objects; reading the source ODB directly
        // is the same set of bytes with one less moving part.
        let imported_objects = entry.objects.import_from_repo(&cloned).await?;
        tracing::debug!(
            "git.remote_clone imported {imported_objects} objects into '{mount_owned}'"
        );

        let tip_sha = tip.id().to_string();
        entry.db.set_ref("HEAD", &format!("refs/heads/{target_branch}"), true).await?;
        entry.db.set_ref(&format!("refs/heads/{target_branch}"), &tip_sha, false).await?;

        // Working tree files into the volume, one by one, isolating failures so a
        // single unwritable path does not abort the whole import.
        let client = state.stores.client(&mount_owned).await?;
        let tree = tip.tree().map_err(|e| git_err("clone tree", e))?;
        let mut files = Vec::new();
        collect_blobs(&cloned, &tree, "", &mut files)?;

        // Charge the whole import against the session quota BEFORE writing anything.
        // Writing first and charging per file would leave a half populated volume when
        // the budget runs out; a clone either fits or is refused cleanly. Importing a
        // large repository therefore needs safety.write_quota_bytes raised, which is
        // the honest trade: a bulk write is still a write.
        let total_bytes: i64 = files
            .iter()
            .filter_map(|(_, oid)| cloned.find_blob(*oid).ok().map(|b| b.size() as i64))
            .sum();
        state.safety.charge_write(&person_owned, &mount_owned, total_bytes)?;

        let mut imported = 0usize;
        let mut skipped: Vec<String> = Vec::new();
        for (rel, oid) in files {
            let path = format!("/{rel}");
            match write_one(&cloned, &client, &path, oid).await {
                Ok(()) => imported += 1,
                Err(e) => skipped.push(format!("{path}: {}", e.message)),
            }
        }

        state.safety.record_audit(
            &person_owned,
            &mount_owned,
            "git.remote_clone",
            "/",
            &format!("{imported} files, {total_bytes} bytes from {url_owned}"),
        );

        // Commit count for the summary: breadth first over the parent graph.
        let mut seen = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        seen.insert(tip.id());
        queue.push_back(tip.id());
        let mut commits_imported = 0usize;
        while let Some(id) = queue.pop_front() {
            commits_imported += 1;
            if let Ok(c) = cloned.find_commit(id) {
                for p in c.parent_ids() {
                    if seen.insert(p) {
                        queue.push_back(p);
                    }
                }
            }
        }

        let mut out = serde_json::Map::new();
        out.insert("mount_id".into(), json!(mount_owned));
        out.insert("url".into(), json!(url_owned));
        out.insert("branch".into(), json!(target_branch));
        out.insert("commit".into(), json!(short(&tip_sha)));
        out.insert("commit_message".into(), json!(tip.message().unwrap_or_default().trim()));
        out.insert("files_imported".into(), json!(imported));
        out.insert("commits_imported".into(), json!(commits_imported));
        out.insert("depth".into(), if depth > 0 { json!(depth) } else { json!("full") });
        out.insert("auth".into(), json!(auth));
        if !skipped.is_empty() {
            out.insert("skipped".into(), json!(skipped));
        }
        Ok(Value::Object(out))
    })
    .await
    // `tmp` is dropped here, removing the temporary clone whatever happened.
}

// ── libgit2 helpers ─────────────────────────────────────────────────────────

fn commit_json(c: &git2::Commit<'_>) -> Value {
    let sha = c.id().to_string();
    let author = c.author();
    let when = author.when();
    json!({
        "sha": sha,
        "short_sha": short(&sha),
        "message": c.message().unwrap_or_default().trim(),
        "author": author.name().unwrap_or_default(),
        "author_email": author.email().unwrap_or_default(),
        "timestamp": when.seconds(),
        "date": format_git_time(when, "%Y-%m-%d %H:%M:%S"),
        "parents": c.parent_ids().map(|p| short(&p.to_string())).collect::<Vec<_>>(),
    })
}

/// Format a git timestamp in the commit's own timezone, like the C# rendering of
/// a `DateTimeOffset`.
fn format_git_time(t: git2::Time, fmt: &str) -> String {
    let offset = FixedOffset::east_opt(t.offset_minutes() * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset is valid"));
    match DateTime::from_timestamp(t.seconds(), 0) {
        Some(dt) => dt.with_timezone(&offset).format(fmt).to_string(),
        None => String::new(),
    }
}

fn commit_touches_path(repo: &Repository, commit: &git2::Commit<'_>, path: &str) -> Result<bool> {
    let rel = path.trim_start_matches('/');
    let tree = commit.tree().map_err(|e| git_err("commit tree", e))?;
    let Some(parent) = commit.parents().next() else {
        return Ok(tree.get_path(Path::new(rel)).is_ok());
    };
    let parent_tree = parent.tree().map_err(|e| git_err("parent tree", e))?;
    let diff = repo
        .diff_tree_to_tree(Some(&parent_tree), Some(&tree), None)
        .map_err(|e| git_err("diff", e))?;
    Ok(diff.deltas().any(|d| {
        let touched = d
            .new_file()
            .path()
            .or_else(|| d.old_file().path())
            .map(|p| p.to_string_lossy().to_string());
        match touched {
            Some(p) => p == rel || p.starts_with(&format!("{rel}/")),
            None => false,
        }
    }))
}

/// Unified diff text, the C# `Patch.Content`.
fn generate_diff(
    repo: &Repository,
    old_tree: Option<&Tree<'_>>,
    new_tree: Option<&Tree<'_>>,
    path: Option<&str>,
) -> Result<String> {
    let mut opts = DiffOptions::new();
    opts.context_lines(DIFF_CONTEXT_LINES);
    if let Some(p) = path {
        opts.pathspec(p.trim_start_matches('/'));
    }
    let diff = repo
        .diff_tree_to_tree(old_tree, new_tree, Some(&mut opts))
        .map_err(|e| git_err("diff", e))?;

    let mut out = String::new();
    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        // Content lines carry their origin marker separately from the payload;
        // headers and hunk headers already contain their own text.
        if matches!(line.origin(), '+' | '-' | ' ') {
            out.push(line.origin());
        }
        out.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .map_err(|e| git_err("diff print", e))?;
    Ok(out)
}

/// One directory of a tree under construction.
#[derive(Default)]
struct DirNode {
    files: BTreeMap<String, Oid>,
    dirs: BTreeMap<String, DirNode>,
}

impl DirNode {
    fn insert(&mut self, rel: &str, oid: Oid) {
        match rel.split_once('/') {
            None => {
                self.files.insert(rel.to_string(), oid);
            }
            Some((head, rest)) => {
                self.dirs.entry(head.to_string()).or_default().insert(rest, oid);
            }
        }
    }
}

/// Snapshot every file of the volume as a git tree. Blobs are written through
/// libgit2 (they are imported into the blob store by the caller).
async fn build_tree_from_volume(repo: &Repository, client: &VolumeClient) -> Result<Oid> {
    let mut root = DirNode::default();
    for (dir, _subdirs, names) in client.walk("/").await? {
        for name in names {
            let full = format!("{}/{name}", dir.trim_end_matches('/'));
            let data = client.read_bytes(&full).await?;
            let oid = repo.blob(&data).map_err(|e| git_err("write blob", e))?;
            root.insert(full.trim_start_matches('/'), oid);
        }
    }
    write_tree(repo, &root)
}

fn write_tree(repo: &Repository, node: &DirNode) -> Result<Oid> {
    let mut builder = repo.treebuilder(None).map_err(|e| git_err("treebuilder", e))?;
    for (name, oid) in &node.files {
        builder
            .insert(name.as_str(), *oid, MODE_FILE)
            .map_err(|e| git_err("tree insert", e))?;
    }
    for (name, sub) in &node.dirs {
        let sub_oid = write_tree(repo, sub)?;
        builder
            .insert(name.as_str(), sub_oid, MODE_DIR)
            .map_err(|e| git_err("tree insert", e))?;
    }
    builder.write().map_err(|e| git_err("tree write", e))
}

/// Depth first walk of a tree, collecting `(relative path, blob oid)`.
fn collect_blobs(
    repo: &Repository,
    tree: &Tree<'_>,
    prefix: &str,
    out: &mut Vec<(String, Oid)>,
) -> Result<()> {
    for e in tree.iter() {
        let name = e.name().unwrap_or_default();
        let full = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        match e.kind() {
            Some(git2::ObjectType::Blob) => out.push((full, e.id())),
            Some(git2::ObjectType::Tree) => {
                // Look the subtree up: the entry alone does not load it.
                let sub = repo.find_tree(e.id()).map_err(|err| git_err("find subtree", err))?;
                collect_blobs(repo, &sub, &full, out)?;
            }
            // Submodules (commit entries) have no bytes to copy.
            _ => {}
        }
    }
    Ok(())
}

async fn write_one(
    repo: &Repository,
    client: &VolumeClient,
    path: &str,
    oid: Oid,
) -> Result<()> {
    let blob = repo.find_blob(oid).map_err(|e| git_err("read blob", e))?;
    if let Some(parent) = crate::util::PosixPath::parent_of(path)
        && parent != "/"
    {
        client.makedirs(&parent, true).await?;
    }
    client.write_bytes_atomic(path, blob.content()).await
}

/// Clone into a real directory: libgit2 needs a filesystem to clone into.
fn clone_to_temp(
    url: &str,
    into: &Path,
    branch: Option<&str>,
    depth: i64,
    token: Option<String>,
) -> Result<Repository> {
    let mut callbacks = git2::RemoteCallbacks::new();
    if let Some(t) = token {
        // The provider expects the token as the password; "oauth2" is the
        // conventional username for both GitHub and GitLab.
        callbacks.credentials(move |_url, _user, _types| {
            git2::Cred::userpass_plaintext("oauth2", &t)
        });
    }
    let mut fetch = git2::FetchOptions::new();
    fetch.remote_callbacks(callbacks);
    if depth > 0 {
        fetch.depth(depth.min(i32::MAX as i64) as i32);
    }

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch);
    if let Some(b) = branch {
        builder.branch(b);
    }
    builder.clone(url, into).map_err(|e| {
        // The message may name the URL but never the token: git2 does not echo
        // credentials, and the token never appears in the URL we pass.
        ToolError::internal(format!("clone failed: {e} (see server logs for details)"))
    })
}

/// A temporary clone directory, removed on drop whatever the outcome.
struct TempClone {
    path: std::path::PathBuf,
}

impl TempClone {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir()
            .join(format!("mcpfs-clone-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempClone {
    fn drop(&mut self) {
        // Git object files are read only, which blocks removal on Windows, so
        // clear the flag on a second pass before giving up.
        if std::fs::remove_dir_all(&self.path).is_err() {
            clear_readonly(&self.path);
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn clear_readonly(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            clear_readonly(&p);
        } else if let Ok(meta) = std::fs::metadata(&p) {
            let mut perms = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(&p, perms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::admin::test_support::{ADMIN, Fixture};
    use super::*;
    use crate::errors::code;
    use crate::git::OAuthTokenStore;

    const OWNER: &str = "owner@test.com";
    const MOUNT: &str = "gitproj";

    struct Env {
        f: Fixture,
        reg: ToolRegistry,
        git: Arc<GitRepoStore>,
        tokens: Arc<OAuthTokenStore>,
    }

    impl Env {
        async fn new() -> Env {
            Env::build(|c| c.git.enabled = true).await
        }

        /// An environment with a tight write quota, to prove the writes are charged.
        async fn with_quota(bytes: i64) -> Env {
            Env::build(move |c| {
                c.git.enabled = true;
                c.safety.write_quota_bytes = bytes;
            })
            .await
        }

        async fn build(tweak: impl FnOnce(&mut crate::config::ServerConfig)) -> Env {
            let f = Fixture::with_config(tweak).await;
            f.seed_project(MOUNT, OWNER).await;
            let git = Arc::new(GitRepoStore::new(f.state.config.clone()));
            let tokens = Arc::new(OAuthTokenStore::new());
            let mut reg = ToolRegistry::new();
            register_with(&mut reg, Some(git.clone()), Some(tokens.clone()));
            Env { f, reg, git, tokens }
        }

        async fn call(&self, name: &str, args: Value) -> Result<Value> {
            self.f.call(&self.reg, OWNER, name, args).await
        }

        async fn as_person(&self, person: &str, name: &str, args: Value) -> Result<Value> {
            self.f.call(&self.reg, person, name, args).await
        }

        async fn write(&self, path: &str, content: &str) {
            let client = self.f.state.stores.client(MOUNT).await.unwrap();
            client.write_text_atomic(path, content).await.unwrap();
        }

        async fn read(&self, path: &str) -> String {
            let client = self.f.state.stores.client(MOUNT).await.unwrap();
            client.read_text(path).await.unwrap()
        }

        /// init + one commit, returning its sha.
        async fn commit(&self, message: &str) -> String {
            self.call("git.commit", json!({"mount_id": MOUNT, "message": message}))
                .await
                .unwrap()["commit_sha"]
                .as_str()
                .unwrap()
                .to_string()
        }
    }

    const ALL_GIT_TOOLS: [&str; 11] = [
        "git.init",
        "git.status",
        "git.branches",
        "git.tags",
        "git.log",
        "git.show",
        "git.diff",
        "git.commit",
        "git.checkout_file",
        "git.blame",
        "git.remote_clone",
    ];

    #[test]
    fn every_git_tool_is_registered() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        assert_eq!(r.len(), 11);
        for name in ALL_GIT_TOOLS {
            assert!(r.resolve(name).is_some(), "{name} is missing");
        }
    }

    #[test]
    fn git_log_schema_matches_the_contract() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.log").unwrap().schema;
        assert_eq!(s.description, "List commits. ref_name defaults to HEAD.");
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"},
                 "ref_name":{"description":"Ref, branch, tag, or commit to start from; defaults to HEAD.","type":"string","default":null},
                 "limit":{"description":"Maximum number of commits to return.","type":"integer","default":20},
                 "path":{"description":"Optional path filter; only commits touching it are returned.","type":"string","default":null}},
               "required":["mount_id"]}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);
    }

    #[test]
    fn git_remote_clone_schema_matches_the_contract() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.remote_clone").unwrap().schema;
        assert!(s.description.starts_with(
            "Clone a remote git repository (GitHub, GitLab, or any HTTPS URL) into a volume. \
             Uses the OAuth token stored by git.auth "
        ));
        assert!(s.description.ends_with("Use depth=1 for a shallow clone (faster on large repos)."));
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the clone is imported into.","type":"string"},
                 "url":{"description":"Remote git repository URL (GitHub, GitLab, or any HTTPS URL).","type":"string"},
                 "branch":{"description":"Branch to clone; omit to use the remote default branch.","type":"string","default":null},
                 "depth":{"description":"Shallow clone depth; 0 clones the full history.","type":"integer","default":0}},
               "required":["mount_id","url"]}"#,
        )
        .unwrap();
        assert_eq!(s.input_schema(), expected);
    }

    #[test]
    fn git_diff_and_blame_schemas_match_the_contract() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let d = &r.resolve("git.diff").unwrap().schema;
        assert_eq!(d.description, "Show diff between two refs or a ref and working tree.");
        assert_eq!(d.input_schema()["required"], json!(["mount_id", "from_ref"]));
        assert_eq!(
            d.input_schema()["properties"]["to_ref"]["description"],
            "Target ref or commit to diff to; omit to diff against the working tree."
        );
        assert_eq!(d.input_schema()["properties"]["to_ref"]["default"], Value::Null);

        let b = &r.resolve("git.blame").unwrap().schema;
        assert_eq!(b.description, "Show who last modified each line of a file.");
        assert_eq!(
            b.input_schema()["properties"]["path"]["description"],
            "Absolute POSIX path of the file to blame."
        );
        assert_eq!(b.input_schema()["required"], json!(["mount_id", "path"]));
    }

    #[test]
    fn simple_git_schemas_take_only_a_mount_id() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        for (name, desc) in [
            ("git.init", "Initialize the volume as a git repository."),
            ("git.status", "Show HEAD, current branch, and all refs."),
            ("git.branches", "List all branches with their SHA."),
            ("git.tags", "List all tags."),
        ] {
            let s = &r.resolve(name).unwrap().schema;
            assert_eq!(s.description, desc);
            let expected: Value = serde_json::from_str(
                r#"{"type":"object","properties":{
                     "mount_id":{"description":"Project/volume id the operation targets.","type":"string"}},
                   "required":["mount_id"]}"#,
            )
            .unwrap();
            assert_eq!(s.input_schema(), expected, "{name}");
        }
    }

    #[test]
    fn commit_and_checkout_schemas_match_the_contract() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let c = &r.resolve("git.commit").unwrap().schema;
        assert_eq!(c.description, "Create a commit from the current state of the volume.");
        assert_eq!(c.input_schema()["required"], json!(["mount_id", "message"]));
        assert_eq!(
            c.input_schema()["properties"]["author_email"]["description"],
            "Optional author email; defaults to the caller person id."
        );
        let co = &r.resolve("git.checkout_file").unwrap().schema;
        assert_eq!(co.description, "Restore a file from a commit into the volume.");
        assert_eq!(co.input_schema()["required"], json!(["mount_id", "commit_sha", "path"]));
        let sh = &r.resolve("git.show").unwrap().schema;
        assert_eq!(sh.input_schema()["properties"]["commit_sha"]["description"],
                   "Commit SHA to show details and diff for.");
    }

    // ── authorization ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_platform_admin_who_is_not_a_member_is_forbidden_on_every_git_tool() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();

        // The separation of duties that matters: administering the platform does
        // not grant access to a project's source history.
        for (name, args) in [
            ("git.init", json!({"mount_id": MOUNT})),
            ("git.status", json!({"mount_id": MOUNT})),
            ("git.branches", json!({"mount_id": MOUNT})),
            ("git.tags", json!({"mount_id": MOUNT})),
            ("git.log", json!({"mount_id": MOUNT})),
            ("git.show", json!({"mount_id": MOUNT, "commit_sha": "abc"})),
            ("git.diff", json!({"mount_id": MOUNT, "from_ref": "main"})),
            ("git.commit", json!({"mount_id": MOUNT, "message": "x"})),
            ("git.checkout_file", json!({"mount_id": MOUNT, "commit_sha": "abc", "path": "/a"})),
            ("git.blame", json!({"mount_id": MOUNT, "path": "/a"})),
            ("git.remote_clone", json!({"mount_id": MOUNT, "url": "https://example.test/r.git"})),
        ] {
            let err = e.as_person(ADMIN, name, args).await.unwrap_err();
            assert_eq!(err.code, code::FORBIDDEN, "{name} must refuse a non member admin");
            assert!(err.message.contains("is not a member of"), "{name}: {}", err.message);
        }
    }

    #[tokio::test]
    async fn a_non_member_is_forbidden_and_a_member_is_allowed() {
        let e = Env::new().await;
        let err = e.as_person("stranger@test.com", "git.init", json!({"mount_id": MOUNT}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::FORBIDDEN);

        e.f.state.admin.add_member(MOUNT, "member@test.com", OWNER).await.unwrap();
        let out = e.as_person("member@test.com", "git.init", json!({"mount_id": MOUNT}))
            .await
            .unwrap();
        assert_eq!(out["initialized"], true);
    }

    #[tokio::test]
    async fn an_unknown_mount_is_project_not_found() {
        let e = Env::new().await;
        let err = e.call("git.init", json!({"mount_id": "ghost"})).await.unwrap_err();
        assert_eq!(err.code, code::PROJECT_NOT_FOUND);
    }

    // ── lifecycle ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn init_is_idempotent_and_points_head_at_main() {
        let e = Env::new().await;
        let out = e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(
            out,
            json!({"mount_id": MOUNT, "initialized": true, "message": "Git repository initialized"})
        );
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();

        let st = e.call("git.status", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(st["mount_id"], MOUNT);
        assert_eq!(st["branch"], "main");
        assert_eq!(st["head"], Value::Null, "no commit yet");
        assert_eq!(st["refs"], json!([]), "the symbolic HEAD is not listed");
    }

    #[tokio::test]
    async fn every_read_tool_requires_init_first() {
        let e = Env::new().await;
        for (name, args) in [
            ("git.status", json!({"mount_id": MOUNT})),
            ("git.branches", json!({"mount_id": MOUNT})),
            ("git.tags", json!({"mount_id": MOUNT})),
            ("git.log", json!({"mount_id": MOUNT})),
            ("git.show", json!({"mount_id": MOUNT, "commit_sha": "abc"})),
            ("git.diff", json!({"mount_id": MOUNT, "from_ref": "main"})),
            ("git.commit", json!({"mount_id": MOUNT, "message": "x"})),
            ("git.blame", json!({"mount_id": MOUNT, "path": "/a.txt"})),
        ] {
            let err = e.call(name, args).await.unwrap_err();
            assert_eq!(err.code, code::NOT_FOUND, "{name}");
            assert!(
                err.message.contains("git not initialized for mount 'gitproj' (call git.init first)"),
                "{name}: {}",
                err.message
            );
        }
    }

    #[tokio::test]
    async fn log_on_an_empty_repository_is_an_empty_list() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        let out = e.call("git.log", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(out, json!({"mount_id": MOUNT, "commits": []}));
        let b = e.call("git.branches", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(b["branches"], json!([]));
        let t = e.call("git.tags", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(t["tags"], json!([]));
    }

    // ── commit, log, show, diff ─────────────────────────────────────────────

    #[tokio::test]
    async fn commit_then_log_show_and_status_agree() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "hello\n").await;
        e.write("/src/lib.rs", "fn main() {}\n").await;

        let out = e
            .call("git.commit", json!({"mount_id": MOUNT, "message": "first commit"}))
            .await
            .unwrap();
        let sha = out["commit_sha"].as_str().unwrap().to_string();
        assert_eq!(sha.len(), 40);
        assert_eq!(out["message"], "first commit");
        assert_eq!(out["author"], "owner", "the local part of the person id");
        assert!(out["timestamp"].as_i64().unwrap() > 1_600_000_000);

        // status sees the new head on the default branch
        let st = e.call("git.status", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(st["head"], sha);
        assert_eq!(st["branch"], "main");
        assert_eq!(st["refs"], json!([{"name": "refs/heads/main", "sha": sha}]));

        // branches list it too
        let b = e.call("git.branches", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(
            b["branches"],
            json!([{"name": "main", "full_ref": "refs/heads/main", "sha": sha}])
        );

        // log walks it
        let log = e.call("git.log", json!({"mount_id": MOUNT})).await.unwrap();
        let c = &log["commits"][0];
        assert_eq!(c["sha"], sha);
        assert_eq!(c["short_sha"], &sha[..8]);
        assert_eq!(c["message"], "first commit");
        assert_eq!(c["author"], "owner");
        assert_eq!(c["author_email"], OWNER);
        assert_eq!(c["parents"], json!([]));
        assert_eq!(c["date"].as_str().unwrap().len(), 19, "yyyy-MM-dd HH:mm:ss");

        // show renders the whole tree as additions
        let shown = e.call("git.show", json!({"mount_id": MOUNT, "commit_sha": sha})).await.unwrap();
        assert_eq!(shown["commit"]["sha"], sha);
        let diff = shown["diff"].as_str().unwrap();
        assert!(diff.contains("+++ b/a.txt"), "got {diff}");
        assert!(diff.contains("+hello"), "got {diff}");
        assert!(diff.contains("src/lib.rs"), "nested paths must be committed: {diff}");
    }

    #[tokio::test]
    async fn a_custom_author_overrides_the_caller() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "x\n").await;
        let out = e
            .call(
                "git.commit",
                json!({"mount_id": MOUNT, "message": "m", "author_name": "Ada",
                       "author_email": "ada@example.test"}),
            )
            .await
            .unwrap();
        assert_eq!(out["author"], "Ada");
        let log = e.call("git.log", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(log["commits"][0]["author"], "Ada");
        assert_eq!(log["commits"][0]["author_email"], "ada@example.test");
    }

    #[tokio::test]
    async fn a_second_commit_chains_onto_the_first() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "one\n").await;
        let first = e.commit("first").await;
        e.write("/a.txt", "two\n").await;
        let second = e.commit("second").await;
        assert_ne!(first, second);

        let log = e.call("git.log", json!({"mount_id": MOUNT})).await.unwrap();
        let commits = log["commits"].as_array().unwrap();
        assert_eq!(commits.len(), 2, "newest first");
        assert_eq!(commits[0]["sha"], second);
        assert_eq!(commits[0]["parents"], json!([&first[..8]]));
        assert_eq!(commits[1]["sha"], first);

        // limit truncates from the newest end
        let one = e.call("git.log", json!({"mount_id": MOUNT, "limit": 1})).await.unwrap();
        assert_eq!(one["commits"].as_array().unwrap().len(), 1);
        assert_eq!(one["commits"][0]["sha"], second);

        // a diff between the two commits shows the change
        let d = e
            .call("git.diff", json!({"mount_id": MOUNT, "from_ref": first, "to_ref": second}))
            .await
            .unwrap();
        assert_eq!(d["from"], first);
        assert_eq!(d["to"], second);
        let text = d["diff"].as_str().unwrap();
        assert!(text.contains("-one"), "got {text}");
        assert!(text.contains("+two"), "got {text}");
    }

    #[tokio::test]
    async fn log_filters_by_path() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "a\n").await;
        let first = e.commit("touch a").await;
        e.write("/b.txt", "b\n").await;
        let second = e.commit("touch b").await;

        let only_b = e
            .call("git.log", json!({"mount_id": MOUNT, "path": "/b.txt"}))
            .await
            .unwrap();
        let shas: Vec<&str> =
            only_b["commits"].as_array().unwrap().iter().map(|c| c["sha"].as_str().unwrap()).collect();
        assert_eq!(shas, vec![second.as_str()], "only the commit adding b.txt");

        let only_a = e
            .call("git.log", json!({"mount_id": MOUNT, "path": "/a.txt"}))
            .await
            .unwrap();
        let shas: Vec<&str> =
            only_a["commits"].as_array().unwrap().iter().map(|c| c["sha"].as_str().unwrap()).collect();
        assert_eq!(shas, vec![first.as_str()]);
    }

    #[tokio::test]
    async fn log_can_start_from_a_branch_a_sha_or_an_unknown_ref() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "a\n").await;
        let sha = e.commit("one").await;

        for start in ["HEAD", "refs/heads/main", "main", sha.as_str()] {
            let out = e
                .call("git.log", json!({"mount_id": MOUNT, "ref_name": start}))
                .await
                .unwrap();
            assert_eq!(out["commits"][0]["sha"], sha, "starting from {start}");
        }

        let out = e
            .call("git.log", json!({"mount_id": MOUNT, "ref_name": "no-such-branch"}))
            .await
            .unwrap();
        assert_eq!(out["commits"], json!([]), "an unknown ref is an empty log");
    }

    #[tokio::test]
    async fn diff_and_show_report_a_missing_object_clearly() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        let absent = "0".repeat(40);

        let err = e
            .call("git.show", json!({"mount_id": MOUNT, "commit_sha": absent}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
        assert!(err.message.contains(&format!("commit '{absent}' not found")));

        let err = e
            .call("git.diff", json!({"mount_id": MOUNT, "from_ref": "nope-nope"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
        assert!(err.message.contains("ref 'nope-nope' not found"), "got {}", err.message);
    }

    #[tokio::test]
    async fn log_explains_a_ref_pointing_at_a_missing_commit() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        // A ref left behind by an import that copied files but not objects.
        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        entry.db.set_ref("refs/heads/main", &"a".repeat(40), false).await.unwrap();

        let err = e.call("git.log", json!({"mount_id": MOUNT})).await.unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
        assert!(err.message.contains("is not present in the git object store"), "got {}", err.message);
        assert!(err.message.contains("Re-run git.remote_clone"));
    }

    #[tokio::test]
    async fn diff_against_no_target_shows_the_tree_as_removed() {
        // C# parity: to_ref omitted diffs the ref against an empty tree, since a
        // bare repository has no working tree to compare with.
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "content\n").await;
        let sha = e.commit("one").await;

        let d = e.call("git.diff", json!({"mount_id": MOUNT, "from_ref": sha})).await.unwrap();
        assert_eq!(d["to"], Value::Null);
        let text = d["diff"].as_str().unwrap();
        assert!(text.contains("-content"), "got {text}");
    }

    #[tokio::test]
    async fn diff_honours_the_path_filter() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "a1\n").await;
        e.write("/b.txt", "b1\n").await;
        let first = e.commit("first").await;
        e.write("/a.txt", "a2\n").await;
        e.write("/b.txt", "b2\n").await;
        let second = e.commit("second").await;

        let d = e
            .call(
                "git.diff",
                json!({"mount_id": MOUNT, "from_ref": first, "to_ref": second, "path": "/a.txt"}),
            )
            .await
            .unwrap();
        let text = d["diff"].as_str().unwrap();
        assert!(text.contains("a.txt"), "got {text}");
        assert!(!text.contains("b.txt"), "the filter must exclude b.txt: {text}");
    }

    // ── checkout_file ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn checkout_file_restores_a_previous_version() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/src/app.py", "print('v1')\n").await;
        let first = e.commit("v1").await;
        e.write("/src/app.py", "print('v2')\n").await;
        e.commit("v2").await;
        assert_eq!(e.read("/src/app.py").await, "print('v2')\n");

        let out = e
            .call(
                "git.checkout_file",
                json!({"mount_id": MOUNT, "commit_sha": first, "path": "src/app.py"}),
            )
            .await
            .unwrap();
        assert_eq!(out["path"], "/src/app.py", "the path is normalized");
        assert_eq!(out["commit"], first);
        assert_eq!(out["size"], 12);
        assert_eq!(e.read("/src/app.py").await, "print('v1')\n");

        // and the restore is audited
        let audit = e.f.state.safety.audit(OWNER, MOUNT);
        let last = audit.last().unwrap();
        assert_eq!(last.op, "git.checkout_file");
        assert_eq!(last.path, "/src/app.py");
        assert_eq!(last.detail, format!("from {first}"));
    }

    #[tokio::test]
    async fn checkout_file_reports_a_path_absent_from_the_commit() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "a\n").await;
        let sha = e.commit("one").await;

        let err = e
            .call(
                "git.checkout_file",
                json!({"mount_id": MOUNT, "commit_sha": sha, "path": "/missing.txt"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
        assert!(err.message.contains("'/missing.txt' not found in commit"), "got {}", err.message);
    }

    #[tokio::test]
    async fn checkout_file_refuses_a_directory_entry() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/src/a.txt", "a\n").await;
        let sha = e.commit("one").await;

        let err = e
            .call("git.checkout_file", json!({"mount_id": MOUNT, "commit_sha": sha, "path": "/src"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(err.message.contains("is not a file in that commit"));
    }

    // ── blame ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn blame_attributes_each_line() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "one\ntwo\n").await;
        let sha = e.commit("first").await;

        let out = e.call("git.blame", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        assert_eq!(out["path"], "/a.txt");
        let lines = out["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["line"], 1);
        assert_eq!(lines[1]["line"], 2);
        assert_eq!(lines[0]["commit"], &sha[..8]);
        assert_eq!(lines[0]["author"], "owner");
        assert_eq!(lines[0]["email"], OWNER);
        assert_eq!(lines[0]["date"].as_str().unwrap().len(), 10, "yyyy-MM-dd");
    }

    #[tokio::test]
    async fn blame_before_any_commit_is_empty_and_an_unknown_file_is_not_found() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        let out = e.call("git.blame", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        assert_eq!(out, json!({"path": "/a.txt", "lines": []}));

        e.write("/a.txt", "a\n").await;
        e.commit("one").await;
        let err = e
            .call("git.blame", json!({"mount_id": MOUNT, "path": "/never-committed.txt"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
        assert!(err.message.contains("not found in repository history"), "got {}", err.message);
    }

    // ── remote_clone ────────────────────────────────────────────────────────

    /// Build a real local repository to clone from, so no network is involved.
    fn seed_origin(dir: &std::path::Path, payload: &str) -> String {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("README.md"), payload).unwrap();
        let repo = git2::Repository::init(dir).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("README.md")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig =
            git2::Signature::new("Origin", "o@t.com", &git2::Time::new(1_700_000_000, 0)).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial\n", &tree, &[]).unwrap();
        format!("file://{}", dir.display())
    }

    /// A clone is a bulk write, so it is charged against the quota, and it is charged
    /// up front: an import that does not fit must leave the volume untouched rather
    /// than half populated.
    #[tokio::test]
    async fn remote_clone_is_charged_up_front_and_writes_nothing_when_over_quota() {
        let e = Env::with_quota(8).await;
        let url = seed_origin(&e.f.dir.path().join("origin"), "0123456789");

        let err = e
            .call("git.remote_clone", json!({"mount_id": MOUNT, "url": url}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::WRITE_QUOTA_EXCEEDED);

        let client = e.f.state.stores.client(MOUNT).await.unwrap();
        assert!(
            !client.exists("/README.md").await.unwrap(),
            "the import must be refused before any file is written"
        );
    }

    /// The happy path charges the imported bytes and records one audit entry.
    #[tokio::test]
    async fn remote_clone_charges_and_audits_the_import() {
        let e = Env::new().await;
        let url = seed_origin(&e.f.dir.path().join("origin"), "0123456789");
        e.call("git.remote_clone", json!({"mount_id": MOUNT, "url": url})).await.unwrap();

        assert_eq!(e.f.state.safety.bytes_written(OWNER, MOUNT), 10);
        let log = e.f.state.safety.audit(OWNER, MOUNT);
        let entry = log
            .iter()
            .find(|x| x.op == "git.remote_clone")
            .expect("an audit entry for the clone");
        assert!(entry.detail.contains("1 files, 10 bytes"), "got {}", entry.detail);
    }

    /// Restoring a file from history is a write, so it is charged too.
    #[tokio::test]
    async fn checkout_file_is_charged_against_the_quota() {
        let e = Env::with_quota(6).await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "abc\n").await;
        let sha = e.commit("one").await;

        // The seed write goes straight through the client, so it costs nothing: the
        // file is 4 bytes and 6 are allowed, so the first restore fits and the second
        // pushes the total to 8 and is refused.
        let args = json!({"mount_id": MOUNT, "path": "/a.txt", "commit_sha": sha});
        let out = e.call("git.checkout_file", args.clone()).await.unwrap();
        assert_eq!(out["path"], "/a.txt");
        let err = e.call("git.checkout_file", args).await.unwrap_err();
        assert_eq!(err.code, code::WRITE_QUOTA_EXCEEDED);
    }

    #[tokio::test]
    async fn remote_clone_imports_files_history_and_refs() {
        let e = Env::new().await;
        // A real local repository to clone from, so no network is involved.
        let src_dir = e.f.dir.path().join("origin");
        std::fs::create_dir_all(src_dir.join("docs")).unwrap();
        std::fs::write(src_dir.join("README.md"), "# origin\n").unwrap();
        std::fs::write(src_dir.join("docs/guide.md"), "guide\n").unwrap();
        let repo = git2::Repository::init(&src_dir).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("README.md")).unwrap();
        index.add_path(Path::new("docs/guide.md")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::new("Origin", "origin@test.com", &git2::Time::new(1_700_000_000, 0))
            .unwrap();
        let tip = repo.commit(Some("HEAD"), &sig, &sig, "initial\n", &tree, &[]).unwrap();

        let url = format!("file://{}", src_dir.display());
        let out = e
            .call("git.remote_clone", json!({"mount_id": MOUNT, "url": url}))
            .await
            .unwrap();

        assert_eq!(out["mount_id"], MOUNT);
        assert_eq!(out["files_imported"], 2);
        assert_eq!(out["commits_imported"], 1);
        assert_eq!(out["commit"], &tip.to_string()[..8]);
        assert_eq!(out["commit_message"], "initial");
        assert_eq!(out["depth"], "full");
        assert_eq!(out["auth"], "anonymous", "no provider detected in a file:// URL");
        assert!(out.get("skipped").is_none());

        // the files landed in the volume
        assert_eq!(e.read("/README.md").await, "# origin\n");
        assert_eq!(e.read("/docs/guide.md").await, "guide\n");

        // and the history is queryable through the git tools
        let log = e.call("git.log", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(log["commits"][0]["sha"], tip.to_string());
        assert_eq!(log["commits"][0]["author"], "Origin");
        let st = e.call("git.status", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(st["head"], tip.to_string());
        assert_eq!(st["branch"], out["branch"]);
    }

    #[tokio::test]
    async fn remote_clone_of_an_empty_repository_reports_it() {
        let e = Env::new().await;
        let src_dir = e.f.dir.path().join("empty-origin");
        git2::Repository::init(&src_dir).unwrap();
        let url = format!("file://{}", src_dir.display());

        let out = e.call("git.remote_clone", json!({"mount_id": MOUNT, "url": url})).await.unwrap();
        assert_eq!(out["files_imported"], 0);
        assert_eq!(out["message"], "Repository is empty");
    }

    #[tokio::test]
    async fn remote_clone_surfaces_a_failure_without_leaking_the_token() {
        let e = Env::new().await;
        e.tokens
            .store_token(OWNER, "github", "gho_supersecret", vec!["repo".into()],
                         Utc::now() + chrono::Duration::hours(1), None)
            .unwrap();
        let err = e
            .call(
                "git.remote_clone",
                json!({"mount_id": MOUNT, "url": "https://github.com/does-not/exist-mcpfs.git"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INTERNAL_ERROR);
        assert!(err.message.starts_with("clone failed:"), "got {}", err.message);
        assert!(!err.message.contains("gho_supersecret"), "a token must never surface");
    }

    #[tokio::test]
    async fn remote_clone_needs_membership() {
        let e = Env::new().await;
        let err = e
            .as_person("stranger@test.com", "git.remote_clone",
                       json!({"mount_id": MOUNT, "url": "https://example.test/x.git"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::FORBIDDEN);
    }

    // ── helpers ─────────────────────────────────────────────────────────────

    #[test]
    fn short_sha_is_eight_characters_and_tolerates_shorter_input() {
        assert_eq!(short("0123456789abcdef"), "01234567");
        assert_eq!(short("abc"), "abc");
        assert_eq!(short(""), "");
    }

    #[test]
    fn git_time_is_rendered_in_the_commit_timezone() {
        // 1700000000 = 2023-11-14T22:13:20Z; +120 minutes shifts the clock.
        let utc = git2::Time::new(1_700_000_000, 0);
        assert_eq!(format_git_time(utc, "%Y-%m-%d %H:%M:%S"), "2023-11-14 22:13:20");
        let plus_two = git2::Time::new(1_700_000_000, 120);
        assert_eq!(format_git_time(plus_two, "%Y-%m-%d %H:%M:%S"), "2023-11-15 00:13:20");
        assert_eq!(format_git_time(utc, "%Y-%m-%d"), "2023-11-14");
    }

    #[tokio::test]
    async fn resolve_ref_follows_symbolic_refs_and_falls_back_to_short_names() {
        let db = SqliteGitDb::open_in_memory().unwrap();
        let sha = "b".repeat(40);
        db.set_ref("HEAD", "refs/heads/main", true).await.unwrap();
        db.set_ref("refs/heads/main", &sha, false).await.unwrap();
        db.set_ref("refs/tags/v1", &sha, false).await.unwrap();

        assert_eq!(resolve_ref(&db, "HEAD").await.unwrap().unwrap(), sha);
        assert_eq!(resolve_ref(&db, "refs/heads/main").await.unwrap().unwrap(), sha);
        assert_eq!(resolve_ref(&db, "main").await.unwrap().unwrap(), sha);
        assert_eq!(resolve_ref(&db, "v1").await.unwrap().unwrap(), sha);
        assert_eq!(resolve_ref(&db, &sha).await.unwrap().unwrap(), sha);
        assert_eq!(resolve_ref(&db, "no-such-thing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_symbolic_head_with_no_branch_yet_resolves_to_nothing() {
        let db = SqliteGitDb::open_in_memory().unwrap();
        db.set_ref("HEAD", "refs/heads/main", true).await.unwrap();
        assert_eq!(resolve_ref(&db, "HEAD").await.unwrap(), None);
    }

    #[test]
    fn dir_node_nests_by_path_segment() {
        let mut root = DirNode::default();
        let oid = Oid::zero();
        root.insert("a.txt", oid);
        root.insert("src/lib.rs", oid);
        root.insert("src/deep/mod.rs", oid);
        assert_eq!(root.files.len(), 1);
        assert_eq!(root.dirs.len(), 1);
        let src = &root.dirs["src"];
        assert_eq!(src.files.len(), 1);
        assert_eq!(src.dirs["deep"].files.len(), 1);
    }

    #[test]
    fn the_temp_clone_dir_is_removed_on_drop() {
        let path = {
            let t = TempClone::new().unwrap();
            let p = t.path().to_path_buf();
            std::fs::write(p.join("f.txt"), "x").unwrap();
            assert!(p.exists());
            p
        };
        assert!(!path.exists(), "the temp clone must not outlive the call");
    }
}
