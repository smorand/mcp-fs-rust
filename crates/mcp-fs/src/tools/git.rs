//! `git.*` tools: init, status, branches, tags, log, show, diff, commit,
//! checkout_file, blame, remote_clone, remote_push, remote_fetch.
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
use crate::git::db::RelationalGitDb;
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

/// Register the thirteen `git.*` tools (the four `git.auth*`/`git.token_set` ones
/// live in [`super::git_auth`]).
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
            .opt_str_null(
                "ref_name",
                "Ref, branch, tag, or commit to start from; defaults to HEAD.",
            )
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
            .opt_str_null(
                "author_email",
                "Optional author email; defaults to the caller person id.",
            ),
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
                on_git_thread(
                    move || async move { blame(&entry, &norm, ref_name.as_deref()).await },
                )
                .await
            }
        }),
    );

    let g = git.clone();
    let t = tokens.clone();
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
            let (g, t) = (g.clone(), t.clone());
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

    let g = git.clone();
    let t = tokens.clone();
    reg.add(
        ToolSchema::new(
            "git.remote_push",
            "Push a local branch to origin under the same name. Creates the branch on the \
             remote when it is absent there. Fails if the push is not a fast-forward; force \
             is not supported.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("branch", "Local branch to push to origin under the same name."),
        handler(move |ctx: ToolCtx, a| {
            let (g, t) = (g.clone(), t.clone());
            async move {
                let mount_id = a.str("mount_id")?;
                let branch = a.str("branch")?;
                let store = authorize(&ctx, &mount_id, g).await?;
                remote_push(&ctx, store, t, &mount_id, &branch).await
            }
        }),
    );

    let g = git;
    reg.add(
        ToolSchema::new(
            "git.remote_fetch",
            "Fetch objects and update refs/remotes/origin/* from origin. Never advances a \
             local branch and never touches a working tree file. A branch removed on the \
             remote is reported in refs_stale, not pruned locally.",
        )
        .req_str("mount_id", "Project/volume id the operation targets."),
        handler(move |ctx: ToolCtx, a| {
            let (g, t) = (g.clone(), tokens.clone());
            async move {
                let mount_id = a.str("mount_id")?;
                let store = authorize(&ctx, &mount_id, g).await?;
                remote_fetch(&ctx, store, t, &mount_id).await
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
    Ok(injected.unwrap_or_else(|| {
        GitRepoStore::shared(ctx.state.config.clone(), ctx.state.stores.relational().clone())
    }))
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
async fn resolve_ref(db: &RelationalGitDb, ref_or_sha: &str) -> Result<Option<String>> {
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

    let name =
        author_name.unwrap_or_else(|| person.split('@').next().unwrap_or(person).to_string());
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
    let te = tree.get_path(Path::new(rel)).map_err(|_| {
        ToolError::not_found(format!("'{norm}' not found in commit '{commit_sha}'"))
    })?;
    let object = te.to_object(&repo).map_err(|e| git_err("read object", e))?;
    let blob = object.as_blob().ok_or_else(|| {
        ToolError::invalid_argument(format!("'{norm}' is not a file in that commit"))
    })?;
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

/// Resolve a remote clone URL to its credential policy (FR-NEW-006/007/008):
/// validate the URL (FR-NEW-041, FR-NEW-042), extract and lowercase the
/// hostname, and match it against `git.hosts` by exact equality through
/// [`crate::git::remote::resolve_host`], the sole reader of that map. Runs
/// before `remote_clone` opens anything, so every failure below happens
/// before any network call (DEC-010, FR-NEW-007) and before any audit entry is
/// written (FR-NEW-042). [`crate::git::remote::validate_remote_url`] parses the
/// URL exactly once: the returned [`url::Url`] is reused for host extraction
/// rather than parsed a second time.
///
/// A URL whose scheme is validated but carries no host at all cannot occur in
/// practice: `https` (the only scheme [`crate::git::remote::validate_remote_url`]
/// accepts) requires one to parse at all. The `else` branch below exists only
/// because [`url::Url::host_str`] returns an `Option`, not because a validated
/// URL is ever actually hostless.
///
/// A host present in `git.hosts` under provider `anonymous` never triggers a
/// token lookup (FR-NEW-008). Any other resolved provider must have a stored,
/// unexpired token for the caller: both a missing token and an expired one
/// fail loud through [`crate::git::OAuthTokenStore::require_valid_credential`]
/// (FR-NEW-018, FR-NEW-019, FR-NEW-070, DEC-019, DEC-020) rather than silently
/// degrading to anonymous (DEC-010, the defect US-002 removed).
async fn resolve_clone_credential(
    ctx: &ToolCtx,
    tokens: &Option<Arc<crate::git::OAuthTokenStore>>,
    url: &str,
) -> Result<(Option<String>, String)> {
    let parsed = crate::git::remote::validate_remote_url(url)?;
    let Some(host) = parsed.host_str().filter(|h| !h.is_empty()) else {
        return Ok((None, "anonymous".to_string()));
    };
    let host = host.to_ascii_lowercase();

    use crate::git::remote::Provider;
    let provider = crate::git::remote::resolve_host(&host).map_err(|_| {
        ToolError::invalid_argument(format!(
            "host '{host}' is not declared in git.hosts; declare it under git.hosts before \
             cloning from this host"
        ))
    })?;
    if provider == Provider::Anonymous {
        return Ok((None, "anonymous".to_string()));
    }

    let provider_name = match provider {
        Provider::Github => "github",
        Provider::Gitlab => "gitlab",
        Provider::Generic => "generic",
        Provider::Anonymous => unreachable!("handled above"),
    };
    let store = match tokens {
        Some(t) => t.clone(),
        None => {
            super::git_auth::token_store(&ctx.state.config, ctx.state.stores.relational()).await?
        }
    };
    let token = store.require_valid_credential(&ctx.person, provider_name, &host)?;
    Ok((Some(token), provider_name.to_string()))
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
    let (token, auth) = resolve_clone_credential(ctx, &tokens, url).await?;
    clone_and_import(ctx, store, mount_id, url, branch, depth, token, auth).await
}

/// Everything that happens after the URL is validated and a credential (or
/// none) is resolved: the actual clone, object import, ref updates, working
/// tree write and audit entry. Split out from [`remote_clone`] so tests that
/// exercise this import mechanics can call it directly with a local `file://`
/// origin, exactly as [`resolve_clone_credential`]'s own tests already call it
/// directly to exercise host resolution: `file://` cannot reach this code
/// through the registered `git.remote_clone` tool any more (FR-NEW-041
/// rejects it before `remote_clone` ever calls this function), so a test of
/// what happens once import starts has to call it directly.
#[allow(clippy::too_many_arguments)]
async fn clone_and_import(
    ctx: &ToolCtx,
    store: Arc<GitRepoStore>,
    mount_id: &str,
    url: &str,
    branch: Option<String>,
    depth: i64,
    token: Option<String>,
    auth: String,
) -> Result<Value> {
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
        let cloned = crate::git::remote::clone_to_temp(
            &url_owned,
            &tmp_path,
            branch_owned.as_deref(),
            depth,
            token,
        )?;

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
                // FR-NEW-020 is unconditional on "completes successfully", and an
                // empty remote is a successful clone, so the volume still gets a
                // repository and an `origin` row even though there is nothing to
                // import yet.
                if !store.is_initialized(&mount_owned).await {
                    store.init_repo(&mount_owned).await?;
                }
                let entry = store.get_or_open_repo(&mount_owned).await?;
                entry.db.add_remote("origin", &url_owned).await?;
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

        // FR-NEW-020: record the clone URL as `origin`. Push, fetch and pull
        // (US-009 to US-011) have no other source for it; re-cloning replaces
        // the row, since `add_remote` upserts by name.
        entry.db.add_remote("origin", &url_owned).await?;

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

/// Resolve the stored `origin`, then push through the same credential pipeline
/// clone uses (FR-NEW-022): [`resolve_clone_credential`] is called unchanged,
/// so host resolution, URL scheme validation and the token expiry gate are
/// proved once, not twice.
async fn remote_push(
    ctx: &ToolCtx,
    store: Arc<GitRepoStore>,
    tokens: Option<Arc<crate::git::OAuthTokenStore>>,
    mount_id: &str,
    branch: &str,
) -> Result<Value> {
    let origin_url = crate::git::remote::require_origin(&store, mount_id).await?;
    let (token, auth) = resolve_clone_credential(ctx, &tokens, &origin_url).await?;
    push_branch(store, mount_id, branch, &origin_url, token, auth).await
}

/// Everything that happens after the origin URL is known and a credential (or
/// none) is resolved: the branch-exists check (FR-NEW-022, before any network
/// call), the actual push through [`crate::git::remote::push_to_remote`] (the
/// sole caller of that function), and the remote-tracking ref update
/// (FR-NEW-061). Split out from [`remote_push`] exactly like
/// [`clone_and_import`] is split from [`remote_clone`], so a test can exercise
/// real push mechanics against a local bare repository with a `file://` origin:
/// that scheme cannot reach this function through the registered
/// `git.remote_push` tool, since [`resolve_clone_credential`] rejects it first
/// (FR-NEW-041).
///
/// No volume byte is written and no volume file changes (FR-NEW-061), so
/// unlike [`clone_and_import`] this charges no write quota and writes no
/// audit entry, matching `git.commit`: a git-internal ref update is not a
/// write to the abstract filesystem.
async fn push_branch(
    store: Arc<GitRepoStore>,
    mount_id: &str,
    branch: &str,
    origin_url: &str,
    token: Option<String>,
    auth: String,
) -> Result<Value> {
    let entry = store.get_or_open_repo(mount_id).await?;
    let branch_ref = format!("refs/heads/{branch}");
    let Some(local_ref) = entry.db.get_ref(&branch_ref).await? else {
        return Err(ToolError::invalid_argument(format!(
            "branch '{branch}' does not exist locally; git.remote_push can only push a branch \
             that already exists"
        )));
    };
    let local_sha = local_ref.target;

    let (branch_owned, origin_owned, local_sha_owned) =
        (branch.to_string(), origin_url.to_string(), local_sha.clone());
    // Held for the whole hydrate-plus-push, so two concurrent pushes to the
    // same branch cannot interleave (E2E-NEW-084): the same lock `git.commit`
    // holds for the whole of a commit.
    let entry_for_thread = entry.clone();
    let outcome = on_git_thread(move || async move {
        let _write = entry_for_thread.write_lock.lock().await;
        let repo = entry_for_thread.repo.lock().await;
        hydrate(&entry_for_thread, &repo).await?;
        // `hydrate` only exports blob-store objects into the on-disk ODB; the
        // db-tracked branch sha (the authoritative value) still needs to be
        // the on-disk ref libgit2 resolves as the push's source side, since
        // `push_to_remote` names the branch by ref, not by oid.
        let oid = parse_oid(&local_sha_owned)?;
        repo.reference(&format!("refs/heads/{branch_owned}"), oid, true, "mcp-fs git.remote_push")
            .map_err(|e| git_err("sync local branch ref", e))?;
        crate::git::remote::push_to_remote(
            &repo,
            &origin_owned,
            &branch_owned,
            &local_sha_owned,
            token,
        )
    })
    .await?;

    // A refused push returns above via `?`, before this line: nothing here
    // advances the tracking ref for a rejection (FR-NEW-061).
    entry.db.set_ref(&format!("refs/remotes/origin/{branch}"), &outcome.remote_sha, false).await?;

    Ok(json!({
        "branch": branch,
        "created": outcome.created,
        "up_to_date": outcome.up_to_date,
        "remote_sha": outcome.remote_sha,
        "auth": auth,
    }))
}

/// Resolve the stored `origin`, then fetch through the same credential
/// pipeline clone and push use (FR-NEW-026): [`resolve_clone_credential`] is
/// called unchanged, so host resolution, URL scheme validation and the token
/// expiry gate are proved once, not three times. `authorize` in the tool
/// registration handler already ran before this is ever reached, so a non
/// member is refused before `require_origin` even looks for a stored remote.
async fn remote_fetch(
    ctx: &ToolCtx,
    store: Arc<GitRepoStore>,
    tokens: Option<Arc<crate::git::OAuthTokenStore>>,
    mount_id: &str,
) -> Result<Value> {
    let origin_url = crate::git::remote::require_origin(&store, mount_id).await?;
    let (token, auth) = resolve_clone_credential(ctx, &tokens, &origin_url).await?;
    fetch_branch(store, mount_id, &origin_url, token, auth).await
}

/// Everything that happens after the origin URL is known and a credential (or
/// none) is resolved: the actual fetch through
/// [`crate::git::remote::fetch_from_remote`] (the sole caller of that
/// function), importing the newly downloaded objects into the blob store, and
/// the `refs/remotes/origin/*` update (FR-NEW-026, FR-NEW-050). Split out from
/// [`remote_fetch`] exactly like [`push_branch`] is split from [`remote_push`],
/// so a test can exercise real fetch mechanics against a local bare repository
/// with a `file://` origin: that scheme cannot reach this function through the
/// registered `git.remote_fetch` tool, since [`resolve_clone_credential`]
/// rejects it first (FR-NEW-041).
///
/// No volume byte is written and no volume file changes (FR-NEW-027): a fetch
/// only ever moves `refs/remotes/origin/*`, so unlike [`clone_and_import`] this
/// charges no write quota and writes no audit entry, matching [`push_branch`].
async fn fetch_branch(
    store: Arc<GitRepoStore>,
    mount_id: &str,
    origin_url: &str,
    token: Option<String>,
    auth: String,
) -> Result<Value> {
    let entry = store.get_or_open_repo(mount_id).await?;

    // The db-tracked view of `refs/remotes/origin/*` before this fetch runs, so
    // a branch present here but absent from the remote's advertisement can be
    // reported as stale rather than silently pruned (DEC-037, FR-NEW-056).
    let before: BTreeMap<String, String> = entry
        .db
        .list_refs()
        .await?
        .into_iter()
        .filter(|r| !r.symbolic && r.name.starts_with("refs/remotes/origin/"))
        .map(|r| (r.name, r.target))
        .collect();

    let origin_owned = origin_url.to_string();
    // Held for the whole hydrate-plus-fetch, the same lock `push_branch` and
    // `git.commit` hold, so a concurrent write to this repository's on disk
    // state cannot interleave with this one.
    let entry_for_thread = entry.clone();
    let outcome = on_git_thread(move || async move {
        let _write = entry_for_thread.write_lock.lock().await;
        let repo = entry_for_thread.repo.lock().await;
        hydrate(&entry_for_thread, &repo).await?;
        let outcome = crate::git::remote::fetch_from_remote(&repo, &origin_owned, token)?;
        entry_for_thread.objects.import_from_repo(&repo).await?;
        Ok(outcome)
    })
    .await?;

    let mut refs_updated = Vec::with_capacity(outcome.refs_updated.len());
    for u in &outcome.refs_updated {
        entry.db.set_ref(&u.ref_name, &u.new_sha, false).await?;
        refs_updated.push(json!({
            "ref": u.ref_name,
            "old_sha": u.old_sha,
            "new_sha": u.new_sha,
        }));
    }

    let advertised: std::collections::HashSet<String> =
        outcome.advertised_branches.iter().map(|b| format!("refs/remotes/origin/{b}")).collect();
    let refs_stale: Vec<String> =
        before.keys().filter(|name| !advertised.contains(name.as_str())).cloned().collect();

    Ok(json!({
        "refs_updated": refs_updated,
        "refs_stale": refs_stale,
        "objects_fetched": outcome.objects_fetched,
        "up_to_date": refs_updated.is_empty(),
        "auth": auth,
    }))
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
        builder.insert(name.as_str(), *oid, MODE_FILE).map_err(|e| git_err("tree insert", e))?;
    }
    for (name, sub) in &node.dirs {
        let sub_oid = write_tree(repo, sub)?;
        builder.insert(name.as_str(), sub_oid, MODE_DIR).map_err(|e| git_err("tree insert", e))?;
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

async fn write_one(repo: &Repository, client: &VolumeClient, path: &str, oid: Oid) -> Result<()> {
    let blob = repo.find_blob(oid).map_err(|e| git_err("read blob", e))?;
    if let Some(parent) = crate::util::PosixPath::parent_of(path)
        && parent != "/"
    {
        client.makedirs(&parent, true).await?;
    }
    client.write_bytes_atomic(path, blob.content()).await
}

/// A temporary clone directory, removed on drop whatever the outcome.
struct TempClone {
    path: std::path::PathBuf,
}

impl TempClone {
    fn new() -> Result<Self> {
        let path =
            std::env::temp_dir().join(format!("mcpfs-clone-{}", uuid::Uuid::new_v4().simple()));
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

        /// An environment with a published `git.hosts` map (US-002). Callers must
        /// hold `crate::git::remote::tests::lock_for_test()` for the whole test,
        /// since the map is a single process wide `OnceLock`.
        async fn with_hosts(pairs: &[(&str, &str)]) -> Env {
            let owned: Vec<(String, String)> =
                pairs.iter().map(|(h, p)| (h.to_string(), p.to_string())).collect();
            Env::build(move |c| {
                c.git.enabled = true;
                c.git.hosts.0 = owned;
                crate::git::remote::validate_hosts(&c.git).expect("a valid reference map");
            })
            .await
        }

        async fn build(tweak: impl FnOnce(&mut crate::config::ServerConfig)) -> Env {
            let f = Fixture::with_config(tweak).await;
            f.seed_project(MOUNT, OWNER).await;
            let git = Arc::new(GitRepoStore::new(
                f.state.config.clone(),
                crate::storage::test_registry(),
            ));
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

    const ALL_GIT_TOOLS: [&str; 13] = [
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
        "git.remote_push",
        "git.remote_fetch",
    ];

    #[test]
    fn every_git_tool_is_registered() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        assert_eq!(r.len(), 13);
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
        assert!(
            s.description.ends_with("Use depth=1 for a shallow clone (faster on large repos).")
        );
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
    fn git_remote_fetch_schema_matches_the_contract() {
        let mut r = ToolRegistry::new();
        register(&mut r);
        let s = &r.resolve("git.remote_fetch").unwrap().schema;
        let expected: Value = serde_json::from_str(
            r#"{"type":"object","properties":{
                 "mount_id":{"description":"Project/volume id the operation targets.","type":"string"}},
               "required":["mount_id"]}"#,
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
        assert_eq!(
            sh.input_schema()["properties"]["commit_sha"]["description"],
            "Commit SHA to show details and diff for."
        );
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
        let err = e
            .as_person("stranger@test.com", "git.init", json!({"mount_id": MOUNT}))
            .await
            .unwrap_err();
        assert_eq!(err.code, code::FORBIDDEN);

        e.f.state.admin.add_member(MOUNT, "member@test.com", OWNER).await.unwrap();
        let out =
            e.as_person("member@test.com", "git.init", json!({"mount_id": MOUNT})).await.unwrap();
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
                err.message
                    .contains("git not initialized for mount 'gitproj' (call git.init first)"),
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
        let shown =
            e.call("git.show", json!({"mount_id": MOUNT, "commit_sha": sha})).await.unwrap();
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

        let only_b = e.call("git.log", json!({"mount_id": MOUNT, "path": "/b.txt"})).await.unwrap();
        let shas: Vec<&str> = only_b["commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["sha"].as_str().unwrap())
            .collect();
        assert_eq!(shas, vec![second.as_str()], "only the commit adding b.txt");

        let only_a = e.call("git.log", json!({"mount_id": MOUNT, "path": "/a.txt"})).await.unwrap();
        let shas: Vec<&str> = only_a["commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["sha"].as_str().unwrap())
            .collect();
        assert_eq!(shas, vec![first.as_str()]);
    }

    #[tokio::test]
    async fn log_can_start_from_a_branch_a_sha_or_an_unknown_ref() {
        let e = Env::new().await;
        e.call("git.init", json!({"mount_id": MOUNT})).await.unwrap();
        e.write("/a.txt", "a\n").await;
        let sha = e.commit("one").await;

        for start in ["HEAD", "refs/heads/main", "main", sha.as_str()] {
            let out =
                e.call("git.log", json!({"mount_id": MOUNT, "ref_name": start})).await.unwrap();
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

        let err =
            e.call("git.show", json!({"mount_id": MOUNT, "commit_sha": absent})).await.unwrap_err();
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
        assert!(
            err.message.contains("is not present in the git object store"),
            "got {}",
            err.message
        );
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
            .call(
                "git.checkout_file",
                json!({"mount_id": MOUNT, "commit_sha": sha, "path": "/src"}),
            )
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

    /// `file://` no longer reaches production code through the registered
    /// `git.remote_clone` tool (FR-NEW-041 rejects it before `remote_clone` ever
    /// calls `clone_and_import`), so every test below that needs a real, local,
    /// no-network origin calls `clone_and_import` directly, exactly like
    /// `resolve_clone_credential`'s own tests already call it directly to
    /// exercise host resolution without going through the full tool.
    async fn call_clone_and_import(e: &Env, url: &str, auth: &str) -> Result<Value> {
        let ctx = e.f.ctx(OWNER);
        clone_and_import(&ctx, e.git.clone(), MOUNT, url, None, 0, None, auth.to_string()).await
    }

    /// A clone is a bulk write, so it is charged against the quota, and it is charged
    /// up front: an import that does not fit must leave the volume untouched rather
    /// than half populated.
    #[tokio::test]
    async fn remote_clone_is_charged_up_front_and_writes_nothing_when_over_quota() {
        let e = Env::with_quota(8).await;
        let url = seed_origin(&e.f.dir.path().join("origin"), "0123456789");

        let err = call_clone_and_import(&e, &url, "anonymous").await.unwrap_err();
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
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();

        assert_eq!(e.f.state.safety.bytes_written(OWNER, MOUNT), 10);
        let log = e.f.state.safety.audit(OWNER, MOUNT);
        let entry =
            log.iter().find(|x| x.op == "git.remote_clone").expect("an audit entry for the clone");
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
        let sig =
            git2::Signature::new("Origin", "origin@test.com", &git2::Time::new(1_700_000_000, 0))
                .unwrap();
        let tip = repo.commit(Some("HEAD"), &sig, &sig, "initial\n", &tree, &[]).unwrap();

        let url = format!("file://{}", src_dir.display());
        let out = call_clone_and_import(&e, &url, "anonymous").await.unwrap();

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

        let out = call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        assert_eq!(out["files_imported"], 0);
        assert_eq!(out["message"], "Repository is empty");

        // FR-NEW-020 is unconditional on "completes successfully": even an empty
        // clone still records origin.
        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        assert_eq!(entry.db.list_remotes().await.unwrap(), vec![("origin".to_string(), url)]);
    }

    /// Also proves E2E-NEW-054: a credential rejected by the remote itself (a
    /// transport-level auth failure, not a local expiry check) does not delete
    /// the token either.
    #[test]
    fn remote_clone_surfaces_a_failure_without_leaking_the_token() {
        // github.com must be declared (US-002, DEC-010): an undeclared host is now
        // a loud config error, not an implicit fallback, so this test's host is no
        // longer free—it must be in git.hosts for the clone to even be attempted.
        with_git_hosts_lock(async {
            let e = Env::with_hosts(&[("github.com", "github")]).await;
            e.tokens
                .store_token(
                    OWNER,
                    "github",
                    "github",
                    "gho_supersecret",
                    vec!["repo".into()],
                    Some(Utc::now() + chrono::Duration::hours(1)),
                    None,
                )
                .await
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

            // E2E-NEW-054: a remote-side rejection never deletes the token.
            let still = e.tokens.get_token(OWNER, "github").unwrap();
            assert_eq!(still.access_token, "gho_supersecret");
        });
    }

    #[tokio::test]
    async fn remote_clone_needs_membership() {
        let e = Env::new().await;
        let err = e
            .as_person(
                "stranger@test.com",
                "git.remote_clone",
                json!({"mount_id": MOUNT, "url": "https://example.test/x.git"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::FORBIDDEN);
    }

    // ── FR-NEW-020: origin persistence ──────────────────────────────────────

    /// E2E-NEW-051: a successful clone records `origin`.
    #[tokio::test]
    async fn e2e_new_051_clone_records_origin() {
        let e = Env::new().await;
        let url = seed_origin(&e.f.dir.path().join("origin"), "hello\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();

        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        assert_eq!(entry.db.list_remotes().await.unwrap(), vec![("origin".to_string(), url)]);
    }

    /// E2E-NEW-052: re-cloning the same volume replaces the `origin` row rather
    /// than adding a second one, since `add_remote` upserts by name.
    #[tokio::test]
    async fn e2e_new_052_re_cloning_replaces_the_origin_row() {
        let e = Env::new().await;
        let url_a = seed_origin(&e.f.dir.path().join("origin-a"), "a\n");
        let url_b = seed_origin(&e.f.dir.path().join("origin-b"), "b\n");
        call_clone_and_import(&e, &url_a, "anonymous").await.unwrap();
        call_clone_and_import(&e, &url_b, "anonymous").await.unwrap();

        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        assert_eq!(entry.db.list_remotes().await.unwrap(), vec![("origin".to_string(), url_b)]);
    }

    // ── FR-NEW-041/042: URL validation, exercised through the registered tool ──

    /// E2E-NEW-142: an HTTPS URL against a declared host passes the scheme
    /// check and proceeds (here, to the next gate: no token is stored, so it
    /// fails at credential resolution rather than at the scheme check, proving
    /// the scheme check itself let it through).
    #[test]
    fn e2e_new_142_an_https_url_is_accepted_and_proceeds_past_the_scheme_check() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let err = e
                .call(
                    "git.remote_clone",
                    json!({"mount_id": MOUNT, "url": "https://github.ibm.com/o/r.git"}),
                )
                .await
                .unwrap_err();
            assert_eq!(
                err.code,
                code::UNAUTHENTICATED,
                "got past the scheme check: {}",
                err.message
            );
        });
    }

    /// E2E-NEW-143/144/145/146: a non-https scheme is rejected before any
    /// network call, naming the scheme; `file://` is rejected too, so no path
    /// on the server's own filesystem is ever read.
    #[tokio::test]
    async fn e2e_new_143_144_145_146_non_https_schemes_are_rejected() {
        let e = Env::new().await;
        for url in [
            "ssh://git@github.com/o/r.git",
            "git://github.com/o/r.git",
            "file:///etc/passwd",
            "file:///tmp/repo",
            "http://github.com/o/r.git",
        ] {
            let err = e
                .call("git.remote_clone", json!({"mount_id": MOUNT, "url": url}))
                .await
                .unwrap_err();
            assert_eq!(err.code, code::INVALID_ARGUMENT, "{url}");
            assert!(
                !e.git.is_initialized(MOUNT).await,
                "{url}: rejected before any repository state is created"
            );
        }
    }

    /// E2E-NEW-147: the scp-style shorthand is rejected as not HTTPS.
    #[tokio::test]
    async fn e2e_new_147_the_scp_shorthand_is_rejected_through_the_tool() {
        let e = Env::new().await;
        let err = e
            .call(
                "git.remote_clone",
                json!({"mount_id": MOUNT, "url": "git@github.com:org/repo.git"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
    }

    /// E2E-NEW-148/149/150: a URL carrying userinfo is rejected before any
    /// network call and before any audit entry, the error names the host only,
    /// and the volume gains no `origin` row and no audit entry mentioning the
    /// credential (it gains no state at all: rejection happens before
    /// `remote_clone` ever calls `clone_and_import`).
    #[tokio::test]
    async fn e2e_new_148_149_150_a_userinfo_url_is_rejected_before_any_state_is_touched() {
        let e = Env::new().await;
        let url = "https://alice:ghp_secret@github.com/o/r.git";
        let err =
            e.call("git.remote_clone", json!({"mount_id": MOUNT, "url": url})).await.unwrap_err();
        assert_eq!(err.code, code::INVALID_ARGUMENT);
        assert!(!err.message.contains("ghp_secret"), "got {}", err.message);
        assert!(!err.message.contains(url), "got {}", err.message);
        assert!(err.message.contains("github.com"), "got {}", err.message);

        assert!(!e.git.is_initialized(MOUNT).await, "no origin can have been recorded");
        let log = e.f.state.safety.audit(OWNER, MOUNT);
        assert!(
            log.iter().all(|x| !x.detail.contains("ghp_secret")),
            "no audit entry may contain the credential"
        );
    }

    // ── FR-NEW-053: `auth` always reports the resolved provider ─────────────

    /// E2E-NEW-194: a generic host's clone response reports `"auth": "generic"`.
    /// A real network clone against a fictional host is not reachable from a
    /// test environment, so this proves the two halves that together produce
    /// the value: `resolve_clone_credential` resolves a generic host's seeded
    /// token to `"generic"` (proven directly by `e2e_new_057_a_generic_host_clones_with_its_token`),
    /// and `clone_and_import` carries whatever `auth` string it is given,
    /// unchanged, into the response.
    #[tokio::test]
    async fn e2e_new_194_auth_reports_generic() {
        let e = Env::new().await;
        let url = seed_origin(&e.f.dir.path().join("origin"), "hi\n");
        let out = call_clone_and_import(&e, &url, "generic").await.unwrap();
        assert_eq!(out["auth"], "generic");
    }

    /// E2E-NEW-195: whatever provider string is resolved is reported verbatim.
    /// Push, fetch and pull don't exist yet (US-009/010/011); once built, they
    /// will report `auth` through this exact same field-setting logic in
    /// `clone_and_import`'s JSON shaping (or an equivalent sharing the same
    /// pattern), so proving it here for all four provider values is proving it
    /// for all four operations.
    #[tokio::test]
    async fn e2e_new_195_auth_is_reported_identically_for_every_provider() {
        let e = Env::new().await;
        for auth in ["github", "gitlab", "generic", "anonymous"] {
            let url = seed_origin(&e.f.dir.path().join(format!("origin-{auth}")), "hi\n");
            let out = call_clone_and_import(&e, &url, auth).await.unwrap();
            assert_eq!(out["auth"], auth, "{auth}");
        }
    }

    /// E2E-NEW-196: an undeclared host and a declared host with no token both
    /// fail outright (already proven individually by `e2e_new_019` and
    /// `e2e_new_053_a_missing_token_fails_and_does_not_fall_back_to_anonymous`),
    /// so there is no `Ok` response either could produce for a false
    /// `"auth": "anonymous"` to hide inside.
    #[test]
    fn e2e_new_196_anonymous_is_never_reported_as_a_fallback() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let ctx = e.f.ctx(OWNER);
            let undeclared = resolve_clone_credential(
                &ctx,
                &Some(e.tokens.clone()),
                "https://git.unknown.test/o/r.git",
            )
            .await;
            assert!(undeclared.is_err());

            let no_token = resolve_clone_credential(
                &ctx,
                &Some(e.tokens.clone()),
                "https://github.ibm.com/org/repo.git",
            )
            .await;
            assert!(no_token.is_err());
        });
    }

    // ── US-002: host resolution by parsing and exact match ─────────────────────
    //
    // Deferred, not implemented here: E2E-NEW-060, E2E-NEW-083 and E2E-NEW-090 all
    // exercise `git.remote_push`, `git.remote_fetch` or `git.remote_pull`, none of
    // which exist in this codebase yet (they belong to later stories US-009,
    // US-010 and US-011). Inventing them is out of this story's scope.
    //
    // Most tests below call `resolve_clone_credential` directly rather than the
    // full `git.remote_clone` tool: it is the exact, sole function `remote_clone`
    // calls for host resolution and credential selection (FR-NEW-006/007/008), so
    // testing it directly exercises the real production code path without a slow
    // or flaky dependency on reachability of a real (and possibly enterprise
    // internal, e.g. `github.ibm.com`) network host.
    //
    // Every test touching the process wide `git.hosts` map is a plain `#[test]`
    // that runs its body through `with_git_hosts_lock`, not `#[tokio::test]`:
    // holding `lock_for_test()`'s `std::sync::MutexGuard` across an `.await`
    // (needed for the whole setup-through-assertions duration, since a
    // concurrently running test could otherwise publish a different map in
    // between) is exactly what clippy's `await_holding_lock` forbids. Running the
    // async body via a private `block_on` instead keeps the guard held across a
    // single synchronous call, not an `.await` expression.

    const REFERENCE_HOSTS: &[(&str, &str)] = &[
        ("github.com", "github"),
        ("github.ibm.com", "github"),
        ("gitlab.acme.corp", "gitlab"),
        ("git.acme.internal", "generic"),
        ("public.example.org", "anonymous"),
    ];

    fn future_expiry() -> DateTime<Utc> {
        Utc::now() + chrono::Duration::hours(1)
    }

    fn past_expiry() -> DateTime<Utc> {
        Utc::now() - chrono::Duration::hours(1)
    }

    /// Serialize a test against every other test touching the global
    /// `git.hosts` map, then run its async body to completion on a fresh,
    /// single test only runtime.
    fn with_git_hosts_lock<F: std::future::Future>(f: F) -> F::Output {
        let _guard = crate::git::remote::tests::lock_for_test();
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    /// E2E-NEW-011: an enterprise GitHub host resolves to `github`, and the
    /// person's stored token for it is the credential offered, not anonymous.
    /// This is the too-narrow half of the substring defect at the old
    /// `git.rs:683-691` (`github.com` only, missing `github.ibm.com`).
    #[test]
    fn e2e_new_011_enterprise_github_host_resolves_and_uses_its_token() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            e.tokens
                .store_token(
                    OWNER,
                    "github",
                    "github",
                    "ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH",
                    vec![],
                    Some(future_expiry()),
                    None,
                )
                .await
                .unwrap();
            let ctx = e.f.ctx(OWNER);
            let (token, auth) = resolve_clone_credential(
                &ctx,
                &Some(e.tokens.clone()),
                "https://github.ibm.com/org/repo.git",
            )
            .await
            .unwrap();
            assert_eq!(auth, "github", "must not be anonymous");
            assert_eq!(token.as_deref(), Some("ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH"));
        });
    }

    /// E2E-NEW-012: a URL whose path merely contains "gitlab" does not resolve to
    /// gitlab, and an undeclared host (`exemple.test`) is rejected even though a
    /// token exists for a different, unrelated declared host
    /// (`gitlab.acme.corp`): that token is never reached, because resolution
    /// fails before any token lookup runs. This is the too-broad half of the same
    /// defect.
    #[test]
    fn e2e_new_012_a_path_containing_gitlab_does_not_resolve_to_gitlab() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            e.tokens
                .store_token(
                    OWNER,
                    "gitlab",
                    "gitlab",
                    "glpat_1111222233334444555566667777",
                    vec![],
                    Some(future_expiry()),
                    None,
                )
                .await
                .unwrap();
            let ctx = e.f.ctx(OWNER);
            let err = resolve_clone_credential(
                &ctx,
                &Some(e.tokens.clone()),
                "https://exemple.test/mirrors/mygitlab-mirror.git",
            )
            .await
            .unwrap_err();
            assert_eq!(err.code, code::INVALID_ARGUMENT);
            assert!(err.message.contains("exemple.test"), "got {}", err.message);
            assert!(!err.message.to_ascii_lowercase().contains("gitlab"), "got {}", err.message);
            // the gitlab.acme.corp token is untouched: still exactly what was stored
            assert_eq!(
                e.tokens.get_token(OWNER, "gitlab").unwrap().access_token,
                "glpat_1111222233334444555566667777"
            );
        });
    }

    /// E2E-NEW-015: host matching is case-insensitive.
    #[test]
    fn e2e_new_015_host_matching_is_case_insensitive() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(&[("github.ibm.com", "github")]).await;
            e.tokens
                .store_token(
                    OWNER,
                    "github",
                    "github",
                    "ghp_case_insensitive",
                    vec![],
                    Some(future_expiry()),
                    None,
                )
                .await
                .unwrap();
            let ctx = e.f.ctx(OWNER);
            let (token, auth) = resolve_clone_credential(
                &ctx,
                &Some(e.tokens.clone()),
                "https://GitHub.IBM.COM/org/repo.git",
            )
            .await
            .unwrap();
            assert_eq!(auth, "github");
            assert_eq!(token.as_deref(), Some("ghp_case_insensitive"));
        });
    }

    /// E2E-NEW-016: near-miss hostnames do not match; only `github.com` is
    /// declared, and none of these resolve to `github`.
    #[test]
    fn e2e_new_016_near_miss_hostnames_do_not_match() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(&[("github.com", "github")]).await;
            let ctx = e.f.ctx(OWNER);
            for bad_host in ["ithub.com", "github.com.evil.test", "notgithub.com", "github.co"] {
                let url = format!("https://{bad_host}/o/r.git");
                let err = resolve_clone_credential(&ctx, &Some(e.tokens.clone()), &url)
                    .await
                    .unwrap_err();
                assert_eq!(err.code, code::INVALID_ARGUMENT, "{bad_host}");
                assert!(err.message.contains(bad_host), "{bad_host}: got {}", err.message);
            }
        });
    }

    /// E2E-NEW-018 (first half): a declared anonymous host resolves with no
    /// token lookup at all. `tokens: &None` (no store available) proves this
    /// structurally: if the anonymous branch tried to look one up it would need
    /// to build the process wide store, which is not reachable here.
    #[test]
    fn e2e_new_018_a_declared_anonymous_host_needs_no_credential() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(&[("public.example.org", "anonymous")]).await;
            let ctx = e.f.ctx(OWNER);
            let (token, auth) =
                resolve_clone_credential(&ctx, &None, "https://public.example.org/o/r.git")
                    .await
                    .unwrap();
            assert_eq!(token, None);
            assert_eq!(auth, "anonymous");
        });
    }

    /// E2E-NEW-018 (second half, superseded by US-008/FR-NEW-041): a hostless
    /// `file:///path` clone can no longer reach the registered `git.remote_clone`
    /// tool at all (the scheme is rejected before `remote_clone` ever calls
    /// `clone_and_import`), so "no credential, clone succeeds, auth reports
    /// anonymous" is now proved at the import-mechanics level instead: a real,
    /// local, no-network clone driven through `clone_and_import` directly, with
    /// `auth` set exactly as `resolve_clone_credential` would resolve it for a
    /// hostless URL, still reports `"auth": "anonymous"`.
    #[tokio::test]
    async fn e2e_new_018_a_hostless_clone_succeeds_and_reports_anonymous() {
        let e = Env::new().await;
        let url = seed_origin(&e.f.dir.path().join("origin"), "hello\n");
        let out = call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        assert_eq!(out["auth"], "anonymous");
    }

    /// E2E-NEW-019: an undeclared host fails before any network activity, naming
    /// the host and stating it must be declared. "Before any network activity" is
    /// a structural guarantee, not a listener assertion: `resolve_clone_credential`
    /// is the very first thing `remote_clone` awaits, returning before
    /// `TempClone::new`, `clone_to_temp` or `on_git_thread` are ever reached.
    #[test]
    fn e2e_new_019_an_undeclared_host_fails_before_any_network_activity() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let err = e
                .call(
                    "git.remote_clone",
                    json!({"mount_id": MOUNT, "url": "https://git.unknown.test/o/r.git"}),
                )
                .await
                .unwrap_err();
            assert_eq!(err.code, code::INVALID_ARGUMENT);
            assert!(err.message.contains("git.unknown.test"), "got {}", err.message);
            assert!(err.message.contains("git.hosts"), "must name git.hosts: got {}", err.message);
            assert!(err.message.to_ascii_lowercase().contains("declared"), "got {}", err.message);
        });
    }

    /// E2E-NEW-053: a missing token fails, and never falls back to anonymous.
    /// FR-NEW-070: the fixed `no token for host` prefix and `ERR_UNAUTHENTICATED`
    /// code (US-005 tightened this from the earlier `ERR_INVALID_ARGUMENT`).
    #[test]
    fn e2e_new_053_a_missing_token_fails_and_does_not_fall_back_to_anonymous() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let err = e
                .call(
                    "git.remote_clone",
                    json!({"mount_id": MOUNT, "url": "https://github.ibm.com/org/repo.git"}),
                )
                .await
                .unwrap_err();
            assert_eq!(err.code, code::UNAUTHENTICATED);
            assert!(
                err.message.starts_with("no token for host github.ibm.com"),
                "got {}",
                err.message
            );
            assert!(
                err.message.to_ascii_lowercase().contains("git.auth")
                    || err.message.to_ascii_lowercase().contains("git.token_set"),
                "must instruct authentication: got {}",
                err.message
            );
            assert!(!err.message.to_ascii_lowercase().contains("anonymous"), "got {}", err.message);
        });
    }

    /// E2E-NEW-095: a clone with an expired token fails before the network,
    /// naming the host and instructing re-authentication.
    ///
    /// *Before US-005, the clone path used `get_token`, which ignores expiry.*
    #[test]
    fn e2e_new_095_clone_with_an_expired_token_fails_before_the_network() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            e.tokens
                .store_token(
                    OWNER,
                    "github",
                    "github",
                    "ghp_expired",
                    vec![],
                    Some(past_expiry()),
                    None,
                )
                .await
                .unwrap();
            let err = e
                .call(
                    "git.remote_clone",
                    json!({"mount_id": MOUNT, "url": "https://github.ibm.com/org/repo.git"}),
                )
                .await
                .unwrap_err();
            assert_eq!(err.code, code::UNAUTHENTICATED);
            assert!(
                err.message.starts_with("token expired for host github.ibm.com"),
                "got {}",
                err.message
            );
            assert!(
                err.message.to_ascii_lowercase().contains("git.auth")
                    || err.message.to_ascii_lowercase().contains("git.token_set"),
                "must instruct authentication: got {}",
                err.message
            );
        });
    }

    /// E2E-NEW-097: an expired token survives the failure: it is still stored,
    /// exactly as seeded, and reads as invalid (never deleted, DEC-020). The
    /// `git.auth_status` half of "reports expired" is US-006's job (the
    /// `expired` label on that surface), out of this story's scope.
    #[test]
    fn e2e_new_097_an_expired_token_survives_the_failure() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            e.tokens
                .store_token(
                    OWNER,
                    "github",
                    "github",
                    "ghp_expired",
                    vec![],
                    Some(past_expiry()),
                    None,
                )
                .await
                .unwrap();
            e.call(
                "git.remote_clone",
                json!({"mount_id": MOUNT, "url": "https://github.ibm.com/org/repo.git"}),
            )
            .await
            .unwrap_err();

            let still = e.tokens.get_token(OWNER, "github").unwrap();
            assert_eq!(still.access_token, "ghp_expired", "the token must survive the failure");
            assert!(!e.tokens.has_valid_token(OWNER, "github"), "still expired, not valid");
        });
    }

    /// E2E-NEW-098: the expiry error and the missing-token error are
    /// machine-distinguishable: same `ERR_UNAUTHENTICATED` code, different
    /// fixed prefix.
    #[test]
    fn e2e_new_098_the_expiry_error_differs_from_the_missing_token_error() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let missing = e
                .call(
                    "git.remote_clone",
                    json!({"mount_id": MOUNT, "url": "https://github.ibm.com/org/repo.git"}),
                )
                .await
                .unwrap_err();

            e.tokens
                .store_token(
                    OWNER,
                    "github",
                    "github",
                    "ghp_expired",
                    vec![],
                    Some(past_expiry()),
                    None,
                )
                .await
                .unwrap();
            let expired = e
                .call(
                    "git.remote_clone",
                    json!({"mount_id": MOUNT, "url": "https://github.ibm.com/org/repo.git"}),
                )
                .await
                .unwrap_err();

            assert_eq!(missing.code, code::UNAUTHENTICATED);
            assert_eq!(expired.code, code::UNAUTHENTICATED);
            assert_ne!(missing.message, expired.message);
            assert!(missing.message.starts_with("no token for host"), "got {}", missing.message);
            assert!(
                expired.message.starts_with("token expired for host"),
                "got {}",
                expired.message
            );
        });
    }

    /// E2E-NEW-099: a token whose `expires_at` equals the current instant is
    /// treated as expired, per `OAuthSession::is_valid_at`'s existing strict
    /// `>` boundary (equal is not valid). By the time the gate re-reads
    /// `Utc::now()` a little later than the moment this test seeded
    /// `expires_at`, the stored instant is provably not in the future, so the
    /// outcome is deterministic rather than a coin flip on clock resolution.
    #[test]
    fn e2e_new_099_a_token_expiring_exactly_now_is_treated_as_expired() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let now = Utc::now();
            e.tokens
                .store_token(OWNER, "github", "github", "ghp_boundary", vec![], Some(now), None)
                .await
                .unwrap();
            let err = e
                .call(
                    "git.remote_clone",
                    json!({"mount_id": MOUNT, "url": "https://github.ibm.com/org/repo.git"}),
                )
                .await
                .unwrap_err();
            assert_eq!(err.code, code::UNAUTHENTICATED);
            assert!(
                err.message.starts_with("token expired for host github.ibm.com"),
                "got {}",
                err.message
            );
        });
    }

    /// E2E-NEW-055: a non-member is refused with ERR_FORBIDDEN before host
    /// resolution ever runs: `authorize` is the very first call `git.remote_clone`
    /// makes, ahead of `remote_clone` and therefore ahead of
    /// `resolve_clone_credential`, so no host resolution, token lookup or
    /// outbound connection occurs.
    #[test]
    fn e2e_new_055_a_non_member_is_refused_before_any_network_call() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let err = e
                .as_person(
                    "bob@test.com",
                    "git.remote_clone",
                    json!({"mount_id": MOUNT, "url": "https://github.ibm.com/org/repo.git"}),
                )
                .await
                .unwrap_err();
            assert_eq!(err.code, code::FORBIDDEN);
        });
    }

    /// E2E-NEW-057: a generic host clones with its stored token, reporting
    /// `auth: "generic"`. `clone_to_temp` (unchanged by this story) always wraps
    /// any resolved token the same way regardless of provider:
    /// `git2::Cred::userpass_plaintext("oauth2", &token)`.
    #[test]
    fn e2e_new_057_a_generic_host_clones_with_its_token() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            e.tokens
                .store_token(
                    OWNER,
                    "generic",
                    "generic",
                    "generic-secret-token",
                    vec![],
                    Some(future_expiry()),
                    None,
                )
                .await
                .unwrap();
            let ctx = e.f.ctx(OWNER);
            let (token, auth) = resolve_clone_credential(
                &ctx,
                &Some(e.tokens.clone()),
                "https://git.acme.internal/o/r.git",
            )
            .await
            .unwrap();
            assert_eq!(auth, "generic");
            assert_eq!(token.as_deref(), Some("generic-secret-token"));
        });
    }

    /// E2E-NEW-058: the undeclared-host error and a malformed-URL error are
    /// distinguishable; the malformed one names malformation, not declaration.
    #[test]
    fn e2e_new_058_undeclared_host_and_malformed_url_are_distinguishable() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let ctx = e.f.ctx(OWNER);
            let undeclared = resolve_clone_credential(
                &ctx,
                &Some(e.tokens.clone()),
                "https://git.unknown.test/o/r.git",
            )
            .await
            .unwrap_err();
            let malformed = resolve_clone_credential(&ctx, &Some(e.tokens.clone()), "not-a-url")
                .await
                .unwrap_err();
            assert_eq!(undeclared.code, code::INVALID_ARGUMENT);
            assert_eq!(malformed.code, code::INVALID_ARGUMENT);
            assert_ne!(undeclared.message, malformed.message);
            assert!(undeclared.message.contains("git.hosts"), "got {}", undeclared.message);
            assert!(!malformed.message.contains("git.hosts"), "got {}", malformed.message);
        });
    }

    /// E2E-NEW-059: a URL with no host is rejected as malformed. The story's own
    /// literal example, `https:///org/repo.git`, does not exercise "no host" in
    /// practice: the `url` crate's WHATWG-compliant lenient slash handling parses
    /// it with host `"org"` (an extra leading slash is skipped, not an error), so
    /// it would actually surface as an *undeclared host* rather than a malformed
    /// one. `https://` (verified against the real `url` crate: `Url::parse`
    /// returns `Err("empty host")`) is the URL that genuinely has no host, so it
    /// is what this test uses; the assertion intent, "a URL with no host fails as
    /// malformed, not as an instruction to declare an empty host", is preserved.
    #[test]
    fn e2e_new_059_a_url_with_no_host_is_rejected_as_malformed() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let ctx = e.f.ctx(OWNER);
            let err = resolve_clone_credential(&ctx, &Some(e.tokens.clone()), "https://")
                .await
                .unwrap_err();
            assert_eq!(err.code, code::INVALID_ARGUMENT);
            assert!(!err.message.contains("git.hosts"), "got {}", err.message);
            assert!(!err.message.contains("declared in"), "got {}", err.message);
        });
    }

    /// E2E-NEW-061: a trailing-dot hostname does not match `github.com`.
    #[test]
    fn e2e_new_061_a_trailing_dot_hostname_does_not_match() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(&[("github.com", "github")]).await;
            let ctx = e.f.ctx(OWNER);
            let err = resolve_clone_credential(
                &ctx,
                &Some(e.tokens.clone()),
                "https://github.com./o/r.git",
            )
            .await
            .unwrap_err();
            assert_eq!(err.code, code::INVALID_ARGUMENT);
        });
    }

    /// E2E-NEW-062: an IDN hostname is resolved on its punycode form. The
    /// story's example map key, `xn--gthub-0na.com`, does not match what the
    /// real `idna`-backed `url` crate actually produces for `gïthub.com`
    /// (verified: `xn--gthub-cta.com`); the map entry here uses the real,
    /// verified encoding so the test proves the real behaviour rather than an
    /// assumed one.
    #[test]
    fn e2e_new_062_an_idn_hostname_is_handled_deterministically() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(&[("xn--gthub-cta.com", "github")]).await;
            e.tokens
                .store_token(
                    OWNER,
                    "github",
                    "github",
                    "idn-token",
                    vec![],
                    Some(future_expiry()),
                    None,
                )
                .await
                .unwrap();
            let ctx = e.f.ctx(OWNER);
            let (token, auth) = resolve_clone_credential(
                &ctx,
                &Some(e.tokens.clone()),
                "https://gïthub.com/o/r.git",
            )
            .await
            .unwrap();
            assert_eq!(auth, "github");
            assert_eq!(token.as_deref(), Some("idn-token"));
        });
    }

    // ── FR-NEW-022/023/024/025/060/061: git.remote_push ─────────────────────

    /// A bare local repository to push into, with one commit on `main`, playing
    /// the role of `origin` in every push test: real git2 push mechanics, no
    /// network involved. Bare, unlike `seed_origin`: pushing into a non-bare
    /// repository's checked-out branch is refused by git itself, which would
    /// test that refusal instead of the one this story owns.
    fn seed_bare_remote(dir: &std::path::Path, payload: &str) -> String {
        let repo = git2::Repository::init_bare(dir).unwrap();
        let sig =
            git2::Signature::new("Origin", "o@t.com", &git2::Time::new(1_700_000_000, 0)).unwrap();
        let blob = repo.blob(payload.as_bytes()).unwrap();
        let mut builder = repo.treebuilder(None).unwrap();
        builder.insert("README.md", blob, MODE_FILE).unwrap();
        let tree_oid = builder.write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("refs/heads/main"), &sig, &sig, "initial\n", &tree, &[]).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        format!("file://{}", dir.display())
    }

    /// One more commit directly on the bare "remote", simulating another writer
    /// having pushed there since the volume last synced: the setup every
    /// non-fast-forward test needs, with no mock, just a second real commit on
    /// the same real repository.
    fn advance_bare_remote(dir: &std::path::Path, branch_ref: &str, payload: &str) -> String {
        let repo = git2::Repository::open_bare(dir).unwrap();
        let parent = repo.find_reference(branch_ref).unwrap().peel_to_commit().unwrap();
        let sig =
            git2::Signature::new("Origin", "o@t.com", &git2::Time::new(1_700_000_100, 0)).unwrap();
        let mut builder = repo.treebuilder(Some(&parent.tree().unwrap())).unwrap();
        let blob = repo.blob(payload.as_bytes()).unwrap();
        builder.insert("EXTRA.md", blob, MODE_FILE).unwrap();
        let tree_oid = builder.write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let oid =
            repo.commit(Some(branch_ref), &sig, &sig, "advance\n", &tree, &[&parent]).unwrap();
        oid.to_string()
    }

    /// The current sha of one ref in the bare "remote" repository, read directly
    /// off disk rather than through anything this story's code touches.
    fn bare_ref_sha(dir: &std::path::Path, name: &str) -> Option<String> {
        let repo = git2::Repository::open_bare(dir).unwrap();
        repo.find_reference(name).ok().and_then(|r| r.target()).map(|oid| oid.to_string())
    }

    /// `file://` cannot reach `push_branch` through the registered
    /// `git.remote_push` tool (`resolve_clone_credential` rejects it first, just
    /// like it does for clone), so every test below that needs a real, local,
    /// no-network push calls `push_branch` directly, exactly like
    /// `call_clone_and_import` does for clone.
    async fn call_push_branch(
        e: &Env,
        origin_url: &str,
        branch: &str,
        auth: &str,
    ) -> Result<Value> {
        push_branch(e.git.clone(), MOUNT, branch, origin_url, None, auth.to_string()).await
    }

    /// E2E-NEW-071: pushing a new local commit updates the remote's existing
    /// branch under the same name, reporting the pushed sha.
    #[tokio::test]
    async fn e2e_new_071_push_updates_an_existing_remote_branch() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-071");
        let url = seed_bare_remote(&remote_dir, "hello\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();

        e.write("/a.txt", "new content\n").await;
        let local_sha = e.commit("second").await;

        let out = call_push_branch(&e, &url, "main", "anonymous").await.unwrap();
        assert_eq!(out["branch"], "main");
        assert_eq!(out["created"], false);
        assert_eq!(out["remote_sha"], local_sha.clone());
        assert_eq!(out["auth"], "anonymous");

        assert_eq!(bare_ref_sha(&remote_dir, "refs/heads/main"), Some(local_sha));
    }

    /// E2E-NEW-072: pushing a branch the remote does not have creates it there,
    /// reporting `created: true`.
    #[tokio::test]
    async fn e2e_new_072_push_creates_a_branch_absent_on_the_remote() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-072");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();

        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        let main_sha = entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target;
        entry.db.set_ref("refs/heads/feature/x", &main_sha, false).await.unwrap();

        let out = call_push_branch(&e, &url, "feature/x", "anonymous").await.unwrap();
        assert_eq!(out["created"], true);
        assert_eq!(out["up_to_date"], false);
        assert_eq!(out["remote_sha"], main_sha.clone());
        assert_eq!(bare_ref_sha(&remote_dir, "refs/heads/feature/x"), Some(main_sha));
    }

    /// E2E-NEW-073: a successful push leaves every volume file, and the local
    /// branch itself, byte-for-byte and sha-for-sha unchanged.
    #[tokio::test]
    async fn e2e_new_073_push_leaves_the_volume_unchanged() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-073");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        e.write("/a.txt", "content\n").await;
        e.commit("second").await;

        let before_readme = e.read("/README.md").await;
        let before_a = e.read("/a.txt").await;
        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        let before_head = entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target;

        call_push_branch(&e, &url, "main", "anonymous").await.unwrap();

        assert_eq!(e.read("/README.md").await, before_readme);
        assert_eq!(e.read("/a.txt").await, before_a);
        let after_head = entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target;
        assert_eq!(after_head, before_head, "refs/heads/main must be unchanged by a push");
    }

    /// E2E-NEW-074 / E2E-NEW-209 / E2E-NEW-210: a push that sends a real update
    /// carries all five response keys correctly valued; pushed again with no
    /// intervening commit, it succeeds idempotently, reporting `up_to_date`.
    #[tokio::test]
    async fn e2e_new_074_209_210_response_shape_and_idempotency() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-074");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        e.write("/a.txt", "v1\n").await;
        let sha = e.commit("second").await;

        let updated = call_push_branch(&e, &url, "main", "anonymous").await.unwrap();
        assert_eq!(
            updated,
            json!({
                "branch": "main", "created": false, "up_to_date": false,
                "remote_sha": sha, "auth": "anonymous",
            })
        );

        let repeated = call_push_branch(&e, &url, "main", "anonymous").await.unwrap();
        assert_eq!(
            repeated,
            json!({
                "branch": "main", "created": false, "up_to_date": true,
                "remote_sha": sha, "auth": "anonymous",
            })
        );
    }

    /// E2E-NEW-076: a non-fast-forward push is refused with a distinct error
    /// naming the branch and stating force is not supported, and the remote's
    /// branch is unchanged.
    #[tokio::test]
    async fn e2e_new_076_a_non_fast_forward_push_is_refused_with_a_distinct_error() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-076");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();

        let advanced_sha = advance_bare_remote(&remote_dir, "refs/heads/main", "extra\n");

        let err = call_push_branch(&e, &url, "main", "anonymous").await.unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);
        assert!(
            err.message.to_ascii_lowercase().contains("non-fast-forward"),
            "got {}",
            err.message
        );
        assert!(err.message.to_ascii_lowercase().contains("force"), "got {}", err.message);
        assert!(err.message.contains("main"), "got {}", err.message);

        assert_eq!(bare_ref_sha(&remote_dir, "refs/heads/main"), Some(advanced_sha));
    }

    /// E2E-NEW-077: a non-fast-forward rejection and a credential rejection
    /// carry different, machine-distinguishable error identities. The
    /// non-fast-forward half is proven directly against a real push in
    /// `e2e_new_076`; this proves the credential half never even reaches
    /// `push_to_remote`, failing instead with a code that is never `NO_CLOBBER`.
    #[test]
    fn e2e_new_077_the_non_fast_forward_error_is_distinguishable_from_an_auth_error() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            e.git.init_repo(MOUNT).await.unwrap();
            let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
            entry.db.add_remote("origin", "https://github.ibm.com/org/repo.git").await.unwrap();
            entry.db.set_ref("refs/heads/main", &"a".repeat(40), false).await.unwrap();
            entry.db.set_ref("HEAD", "refs/heads/main", true).await.unwrap();

            // No token stored for github.ibm.com: credential resolution fails
            // first, before any push is attempted.
            let err = e
                .call("git.remote_push", json!({"mount_id": MOUNT, "branch": "main"}))
                .await
                .unwrap_err();
            assert_eq!(err.code, code::UNAUTHENTICATED);
            assert_ne!(err.code, code::NO_CLOBBER);
        });
    }

    /// E2E-NEW-078: a branch absent locally is rejected, naming it, before any
    /// network call. The origin host is declared anonymous so this test needs
    /// no stored token to reach the branch check.
    #[test]
    fn e2e_new_078_a_branch_absent_locally_is_rejected() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(&[("public.example.org", "anonymous")]).await;
            e.git.init_repo(MOUNT).await.unwrap();
            let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
            entry.db.add_remote("origin", "https://public.example.org/org/repo.git").await.unwrap();

            let err = e
                .call("git.remote_push", json!({"mount_id": MOUNT, "branch": "no-such-branch"}))
                .await
                .unwrap_err();
            assert_eq!(err.code, code::INVALID_ARGUMENT);
            assert!(err.message.contains("no-such-branch"), "got {}", err.message);
        });
    }

    /// E2E-NEW-081: a non-member is refused with `ERR_FORBIDDEN` before host
    /// resolution or any token lookup: `authorize` is the very first call
    /// `git.remote_push` makes. The volume is never even initialized here, so a
    /// forbidden result proves `authorize` ran ahead of `require_origin` too
    /// (which would otherwise fail with `ERR_INVALID_ARGUMENT`).
    #[test]
    fn e2e_new_081_a_non_member_cannot_push() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let err = e
                .as_person(
                    "bob@test.com",
                    "git.remote_push",
                    json!({"mount_id": MOUNT, "branch": "main"}),
                )
                .await
                .unwrap_err();
            assert_eq!(err.code, code::FORBIDDEN);
        });
    }

    /// E2E-NEW-084: two concurrent pushes to the same branch serialize on the
    /// per-repository write lock (`FR-609`): neither corrupts state, and
    /// exactly one of the two observes the update it raced the other for while
    /// the other finds it already applied.
    #[tokio::test]
    async fn e2e_new_084_concurrent_pushes_serialize() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-084");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        e.write("/a.txt", "v\n").await;
        let sha = e.commit("second").await;

        let (r1, r2) = tokio::join!(
            push_branch(e.git.clone(), MOUNT, "main", &url, None, "anonymous".to_string()),
            push_branch(e.git.clone(), MOUNT, "main", &url, None, "anonymous".to_string()),
        );
        let r1 = r1.unwrap();
        let r2 = r2.unwrap();
        for out in [&r1, &r2] {
            assert_eq!(out["remote_sha"], sha.clone());
        }
        let up_to_date_count = [&r1, &r2].iter().filter(|o| o["up_to_date"] == true).count();
        assert_eq!(up_to_date_count, 1, "exactly one of the two racers must see up_to_date");
        assert_eq!(bare_ref_sha(&remote_dir, "refs/heads/main"), Some(sha));
    }

    /// E2E-NEW-085: the remote advancing between the pipeline reading `origin`
    /// and the actual push (the exact race window `remote_push` leaves open
    /// between `require_origin`/`resolve_clone_credential` and `push_branch`)
    /// produces the same non-fast-forward error as a non-racing rejection: the
    /// remote's state at push time is authoritative either way.
    #[tokio::test]
    async fn e2e_new_085_a_push_racing_a_remote_update_fails_cleanly() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-085");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();

        advance_bare_remote(&remote_dir, "refs/heads/main", "raced\n");

        let err = call_push_branch(&e, &url, "main", "anonymous").await.unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER, "the same identity as a non-racing rejection");
    }

    /// E2E-NEW-211: `created` and `up_to_date` are never both true: a branch
    /// created on the remote reports `created: true, up_to_date: false`, and
    /// pushed again unchanged reports `created: false, up_to_date: true`.
    #[tokio::test]
    async fn e2e_new_211_created_and_up_to_date_are_never_both_true() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-211");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        let main_sha = entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target;
        entry.db.set_ref("refs/heads/feature/y", &main_sha, false).await.unwrap();

        let first = call_push_branch(&e, &url, "feature/y", "anonymous").await.unwrap();
        assert_eq!(first["created"], true);
        assert_eq!(first["up_to_date"], false);

        let second = call_push_branch(&e, &url, "feature/y", "anonymous").await.unwrap();
        assert_eq!(second["created"], false);
        assert_eq!(second["up_to_date"], true);
    }

    /// E2E-NEW-212 / E2E-NEW-213: a successful push advances
    /// `refs/remotes/origin/{branch}` to the pushed sha, creating it since
    /// cloning alone never does; `git.status` lists it unfiltered, and
    /// `refs/heads/main` (the local branch itself) is untouched.
    #[tokio::test]
    async fn e2e_new_212_213_a_successful_push_advances_the_remote_tracking_ref() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-212");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();

        assert!(
            entry.db.get_ref("refs/remotes/origin/main").await.unwrap().is_none(),
            "a clone alone must not create a remote-tracking ref"
        );

        e.write("/a.txt", "v1\n").await;
        let sha = e.commit("second").await;
        let out = call_push_branch(&e, &url, "main", "anonymous").await.unwrap();
        assert_eq!(out["remote_sha"], sha.clone());

        let tracking = entry.db.get_ref("refs/remotes/origin/main").await.unwrap().unwrap();
        assert_eq!(tracking.target, sha, "refs/remotes/origin/main must now hold the pushed sha");

        let st = e.call("git.status", json!({"mount_id": MOUNT})).await.unwrap();
        let refs = st["refs"].as_array().unwrap();
        assert!(
            refs.iter().any(|r| r["name"] == "refs/remotes/origin/main" && r["sha"] == sha.clone()),
            "git.status must list the remote-tracking ref: {refs:?}"
        );

        let heads_sha = entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target;
        assert_eq!(heads_sha, sha, "refs/heads/main is set by git.commit, not moved by the push");
    }

    /// E2E-NEW-214: a refused push advances nothing: `refs/remotes/origin/main`
    /// stays exactly where a prior successful push left it.
    #[tokio::test]
    async fn e2e_new_214_a_refused_push_advances_nothing() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-214");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        let first = call_push_branch(&e, &url, "main", "anonymous").await.unwrap();
        let baseline_sha = first["remote_sha"].as_str().unwrap().to_string();

        advance_bare_remote(&remote_dir, "refs/heads/main", "extra\n");
        let err = call_push_branch(&e, &url, "main", "anonymous").await.unwrap_err();
        assert_eq!(err.code, code::NO_CLOBBER);

        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        let tracking = entry.db.get_ref("refs/remotes/origin/main").await.unwrap().unwrap();
        assert_eq!(tracking.target, baseline_sha, "a refused push must not move the tracking ref");
    }

    // ── FR-NEW-026/027/050/056/062: git.remote_fetch ────────────────────────

    /// Create a new ref on the bare "remote" pointing at the same target as
    /// `from_ref`, simulating the remote having gained a branch, with no mock:
    /// a real ref on a real repository.
    fn create_branch_on_bare_remote(dir: &std::path::Path, branch_ref: &str, from_ref: &str) {
        let repo = git2::Repository::open_bare(dir).unwrap();
        let target = repo.find_reference(from_ref).unwrap().target().unwrap();
        repo.reference(branch_ref, target, true, "test branch").unwrap();
    }

    /// Delete a ref on the bare "remote", simulating a branch removed upstream.
    fn delete_ref_on_bare_remote(dir: &std::path::Path, ref_name: &str) {
        let repo = git2::Repository::open_bare(dir).unwrap();
        repo.find_reference(ref_name).unwrap().delete().unwrap();
    }

    /// An annotated tag on the bare "remote", pointing at `target_ref`, to prove
    /// a fetch takes no tags (FR-NEW-062).
    fn tag_bare_remote(dir: &std::path::Path, tag: &str, target_ref: &str) {
        let repo = git2::Repository::open_bare(dir).unwrap();
        let target = repo.find_reference(target_ref).unwrap().peel_to_commit().unwrap();
        let sig =
            git2::Signature::new("Origin", "o@t.com", &git2::Time::new(1_700_000_200, 0)).unwrap();
        repo.tag(tag, target.as_object(), &sig, "v1", false).unwrap();
    }

    /// `file://` cannot reach `fetch_branch` through the registered
    /// `git.remote_fetch` tool (`resolve_clone_credential` rejects it first,
    /// just like it does for clone and push), so every test below that needs a
    /// real, local, no-network fetch calls `fetch_branch` directly, exactly
    /// like `call_push_branch` does for push.
    async fn call_fetch_branch(e: &Env, origin_url: &str, auth: &str) -> Result<Value> {
        fetch_branch(e.git.clone(), MOUNT, origin_url, None, auth.to_string()).await
    }

    /// E2E-NEW-086 / E2E-NEW-087: a fetch against a remote two commits ahead
    /// updates `refs/remotes/origin/main` to the remote tip, reports that ref
    /// and a non-zero object count, and the fetched commit (and its tree and
    /// blobs) are importable through `git.show`.
    #[tokio::test]
    async fn e2e_new_086_087_fetch_updates_refs_and_downloads_objects() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-086");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        advance_bare_remote(&remote_dir, "refs/heads/main", "c1\n");
        let tip = advance_bare_remote(&remote_dir, "refs/heads/main", "c2\n");

        let out = call_fetch_branch(&e, &url, "anonymous").await.unwrap();
        let updates = out["refs_updated"].as_array().unwrap();
        assert!(
            updates.iter().any(|u| u["ref"] == "refs/remotes/origin/main" && u["new_sha"] == tip),
            "got {updates:?}"
        );
        assert!(out["objects_fetched"].as_i64().unwrap() > 0);

        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        assert_eq!(
            entry.db.get_ref("refs/remotes/origin/main").await.unwrap().unwrap().target,
            tip
        );

        let shown =
            e.call("git.show", json!({"mount_id": MOUNT, "commit_sha": tip})).await.unwrap();
        assert_eq!(shown["commit"]["sha"], tip, "git.show must read the fetched commit");
    }

    /// E2E-NEW-088: a fetch against a remote that is ahead leaves every volume
    /// file byte-for-byte identical, and `refs/heads/main` unchanged.
    #[tokio::test]
    async fn e2e_new_088_fetch_leaves_local_refs_and_files_untouched() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-088");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();

        let before_readme = e.read("/README.md").await;
        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        let before_head = entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target;

        advance_bare_remote(&remote_dir, "refs/heads/main", "c1\n");
        advance_bare_remote(&remote_dir, "refs/heads/main", "c2\n");
        call_fetch_branch(&e, &url, "anonymous").await.unwrap();

        assert_eq!(e.read("/README.md").await, before_readme);
        let after_head = entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target;
        assert_eq!(after_head, before_head, "refs/heads/main must be unchanged by a fetch");
    }

    /// E2E-NEW-091: an unreachable remote fails naming the host, and the
    /// failure code is never `ERR_UNAUTHENTICATED`, the code a credential
    /// failure always carries: the two are machine-distinguishable.
    #[tokio::test]
    async fn e2e_new_091_an_unreachable_remote_fails_naming_the_host() {
        let e = Env::new().await;
        e.git.init_repo(MOUNT).await.unwrap();
        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        let repo = entry.repo.lock().await;

        let err = crate::git::remote::fetch_from_remote(
            &repo,
            "https://mcp-fs-fetch-unreachable-test.invalid/o/r.git",
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, code::INTERNAL_ERROR);
        assert_ne!(err.code, code::UNAUTHENTICATED, "distinguishable from a credential failure");
        assert!(
            err.message.contains("mcp-fs-fetch-unreachable-test.invalid"),
            "the host must be named: got {}",
            err.message
        );
    }

    /// E2E-NEW-092 / E2E-NEW-187: a fetch that brings nothing new succeeds,
    /// reports zero refs updated, and `up_to_date` is exactly true.
    #[tokio::test]
    async fn e2e_new_092_187_a_fetch_with_nothing_new_is_idempotent() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-092");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        call_fetch_branch(&e, &url, "anonymous").await.unwrap();

        let out = call_fetch_branch(&e, &url, "anonymous").await.unwrap();
        assert_eq!(out["refs_updated"], json!([]));
        assert_eq!(out["up_to_date"], true);
    }

    /// E2E-NEW-093 / E2E-NEW-199: a branch deleted upstream is reported in
    /// `refs_stale`, its remote-tracking ref stays at its previous sha, and no
    /// local ref or file changes.
    #[tokio::test]
    async fn e2e_new_093_199_a_deleted_upstream_branch_is_reported_not_pruned() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-093");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        create_branch_on_bare_remote(&remote_dir, "refs/heads/feature-x", "refs/heads/main");
        call_fetch_branch(&e, &url, "anonymous").await.unwrap();

        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        let baseline =
            entry.db.get_ref("refs/remotes/origin/feature-x").await.unwrap().unwrap().target;
        let before_readme = e.read("/README.md").await;
        let before_head = entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target;

        delete_ref_on_bare_remote(&remote_dir, "refs/heads/feature-x");
        let out = call_fetch_branch(&e, &url, "anonymous").await.unwrap();

        assert_eq!(out["refs_stale"], json!(["refs/remotes/origin/feature-x"]));
        let tracking =
            entry.db.get_ref("refs/remotes/origin/feature-x").await.unwrap().unwrap().target;
        assert_eq!(tracking, baseline, "a stale tracking ref must not move");
        assert_eq!(e.read("/README.md").await, before_readme);
        assert_eq!(entry.db.get_ref("refs/heads/main").await.unwrap().unwrap().target, before_head);
    }

    /// E2E-NEW-094 / E2E-NEW-186: a branch new on the remote creates only
    /// `refs/remotes/origin/{branch}`, with `old_sha` null since it never
    /// existed before; no `refs/heads/{branch}` and no new volume file appear.
    #[tokio::test]
    async fn e2e_new_094_186_a_new_remote_branch_creates_only_a_tracking_ref() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-094");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        let client = e.f.state.stores.client(MOUNT).await.unwrap();
        let before_listing = client.list_dir("/").await.unwrap();

        create_branch_on_bare_remote(&remote_dir, "refs/heads/feature-y", "refs/heads/main");
        let tip = bare_ref_sha(&remote_dir, "refs/heads/main").unwrap();

        let out = call_fetch_branch(&e, &url, "anonymous").await.unwrap();
        let updates = out["refs_updated"].as_array().unwrap();
        let fy = updates.iter().find(|u| u["ref"] == "refs/remotes/origin/feature-y").unwrap();
        assert_eq!(fy["old_sha"], Value::Null);
        assert_eq!(fy["new_sha"], tip);

        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        assert_eq!(
            entry.db.get_ref("refs/remotes/origin/feature-y").await.unwrap().unwrap().target,
            tip
        );
        assert!(
            entry.db.get_ref("refs/heads/feature-y").await.unwrap().is_none(),
            "no local branch must be created by a fetch"
        );
        let after_listing = client.list_dir("/").await.unwrap();
        assert_eq!(after_listing.len(), before_listing.len(), "no file must appear in the volume");
    }

    /// E2E-NEW-185: the fetch response carries exactly the five declared
    /// fields, valued as the contract requires.
    #[tokio::test]
    async fn e2e_new_185_the_fetch_response_carries_every_declared_field() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-185");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        advance_bare_remote(&remote_dir, "refs/heads/main", "c1\n");
        let tip = advance_bare_remote(&remote_dir, "refs/heads/main", "c2\n");

        let out = call_fetch_branch(&e, &url, "anonymous").await.unwrap();
        assert_eq!(
            out["refs_updated"],
            json!([{"ref": "refs/remotes/origin/main", "old_sha": Value::Null, "new_sha": tip}])
        );
        assert_eq!(out["refs_stale"], json!([]));
        assert_eq!(out["up_to_date"], false);
        assert_eq!(out["auth"], "anonymous");
        assert!(out["objects_fetched"].as_i64().unwrap() > 0);
        let obj = out.as_object().unwrap();
        assert_eq!(obj.len(), 5, "exactly the five declared fields: {obj:?}");
    }

    /// E2E-NEW-215 / E2E-NEW-216: a fetch against a remote holding a branch and
    /// a tag updates and reports only the branch's remote-tracking ref; the
    /// tag is never imported, and `git.tags` is unchanged.
    #[tokio::test]
    async fn e2e_new_215_216_fetch_brings_branches_and_takes_no_tags() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-215");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        let tip = bare_ref_sha(&remote_dir, "refs/heads/main").unwrap();
        tag_bare_remote(&remote_dir, "v1", "refs/heads/main");
        let before_tags = e.call("git.tags", json!({"mount_id": MOUNT})).await.unwrap();

        let out = call_fetch_branch(&e, &url, "anonymous").await.unwrap();
        let updates = out["refs_updated"].as_array().unwrap();
        assert!(
            updates.iter().any(|u| u["ref"] == "refs/remotes/origin/main" && u["new_sha"] == tip),
            "got {updates:?}"
        );
        assert!(
            !updates.iter().any(|u| u["ref"].as_str().unwrap_or_default().contains("tags")),
            "no tag ref may appear in refs_updated: {updates:?}"
        );

        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        assert!(entry.db.get_ref("refs/tags/v1").await.unwrap().is_none(), "no tag imported");
        let after_tags = e.call("git.tags", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(after_tags, before_tags, "git.tags must be unchanged by a fetch");
    }

    /// E2E-NEW-217: `git.remote_pull` (US-011) does not exist yet, and this
    /// story's scope boundary forbids building it here. But FR-NEW-062 states
    /// pull inherits the fetch refspec because its first step IS this fetch,
    /// so the way to prove that now, without building pull, is to call the
    /// exact shared primitive pull will call, `fetch_from_remote`, directly,
    /// exactly as US-011 will: any future caller of this function gets
    /// no-tags-taken behaviour for free, by construction, not by a second
    /// implementation.
    #[tokio::test]
    async fn e2e_new_217_pull_will_inherit_the_fetch_refspec() {
        let e = Env::new().await;
        let remote_dir = e.f.dir.path().join("remote-217");
        let url = seed_bare_remote(&remote_dir, "hi\n");
        call_clone_and_import(&e, &url, "anonymous").await.unwrap();
        tag_bare_remote(&remote_dir, "v1", "refs/heads/main");
        let before_tags = e.call("git.tags", json!({"mount_id": MOUNT})).await.unwrap();

        let entry = e.git.get_or_open_repo(MOUNT).await.unwrap();
        {
            let repo = entry.repo.lock().await;
            hydrate(&entry, &repo).await.unwrap();
            let outcome = crate::git::remote::fetch_from_remote(&repo, &url, None).unwrap();
            assert!(
                outcome.refs_updated.iter().all(|u| !u.ref_name.contains("tags")),
                "got {:?}",
                outcome.refs_updated
            );
        }
        assert!(entry.db.get_ref("refs/tags/v1").await.unwrap().is_none());
        let after_tags = e.call("git.tags", json!({"mount_id": MOUNT})).await.unwrap();
        assert_eq!(after_tags, before_tags);
    }

    /// Not one of this story's 14 owned tests, but a required implementation
    /// property ("Authorization first, before host resolution or token
    /// lookup"): the volume is never even initialized here, so a forbidden
    /// result proves `authorize` ran ahead of `require_origin`, exactly like
    /// `e2e_new_081_a_non_member_cannot_push` proves it for push.
    #[test]
    fn a_non_member_cannot_fetch() {
        with_git_hosts_lock(async {
            let e = Env::with_hosts(REFERENCE_HOSTS).await;
            let err = e
                .as_person("bob@test.com", "git.remote_fetch", json!({"mount_id": MOUNT}))
                .await
                .unwrap_err();
            assert_eq!(err.code, code::FORBIDDEN);
        });
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
        let db = RelationalGitDb::open_in_memory().await.unwrap();
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
        let db = RelationalGitDb::open_in_memory().await.unwrap();
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
