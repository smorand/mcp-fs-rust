//! The negotiation of a fetch: what a client holds decides what the service sends.
//!
//! The contract is a reference, git's own `upload-pack` in stateless mode with
//! `multi_ack_detailed` (SPEC-0067): the same acknowledgement lines in the same order and the
//! same set of objects, except for the declared differences of the spec. A request carries the
//! whole exchange (the wants, then every held commit the client has stated so far, then the end
//! marker or a flush), so one request is enough to answer: the service keeps no state between
//! the requests of one fetch.

use super::pktline::{self, PacketType, PktReader};
use super::{SIDE_BAND_CHUNK, empty_pack};
use crate::errors::{Result, ToolError};
use git2::{ObjectType, Oid, Repository};
use std::collections::HashSet;

/// A parsed `git-upload-pack` request.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// Wanted ids as written, in order; the options after the first id are not kept.
    pub wants: Vec<String>,
    /// Held ids as written, in order, up to the flush or end marker that closes the haves.
    pub haves: Vec<String>,
    /// The end marker was present: the client now expects the objects.
    pub done: bool,
}

/// Read the whole request. A flush that follows the haves, a second flush or the end marker
/// closes it; the flush after the wanted lines does not.
pub fn parse(body: &[u8]) -> Request {
    let mut req = Request::default();
    let mut reader = PktReader::new(body);
    let mut flushes = 0;
    while !reader.remaining().is_empty() {
        let (line, kind) = reader.read_line();
        if kind == PacketType::Flush {
            flushes += 1;
            if flushes > 1 || !req.haves.is_empty() {
                break;
            }
            continue;
        }
        let Some(line) = line else { break };
        if let Some(rest) = line.strip_prefix("want ") {
            // The first want carries the client's options after a space.
            req.wants.push(rest.trim().split(' ').next().unwrap_or("").to_string());
        } else if let Some(rest) = line.strip_prefix("have ") {
            req.haves.push(rest.trim().to_string());
        } else if line == "done" {
            req.done = true;
            break;
        }
    }
    req
}

/// The whole response body of a `git-upload-pack` request.
pub fn answer(repo: &Repository, _ref_values: &[String], req: &Request) -> Result<Vec<u8>> {
    if req.wants.is_empty() {
        return Ok(pktline::flush());
    }
    let mut out = Vec::new();
    // A request that states no held id is a clone, or a client that does not negotiate: one
    // NAK and everything reachable, as before the negotiation existed.
    if req.haves.is_empty() {
        out.extend_from_slice(&pktline::encode("NAK\n"));
        return finish(repo, &req.wants, &[], out);
    }
    let commons = acknowledge(repo, &req.haves, &mut out)?;
    if !req.done {
        // A round that does not end the negotiation: acknowledgements only.
        out.extend_from_slice(&pktline::encode("NAK\n"));
        return Ok(out);
    }
    match commons.last() {
        Some(last) => out.extend_from_slice(&pktline::encode(&format!("ACK {last}\n"))),
        None => out.extend_from_slice(&pktline::encode("NAK\n")),
    }
    finish(repo, &req.wants, &commons, out)
}

/// Append the objects the client lacks, framed on side-band channel 1, and the closing bytes.
fn finish(
    repo: &Repository,
    wants: &[String],
    commons: &[Oid],
    mut out: Vec<u8>,
) -> Result<Vec<u8>> {
    let pack = build_pack(repo, wants, commons)?;
    for chunk in pack.chunks(SIDE_BAND_CHUNK) {
        let mut framed = Vec::with_capacity(chunk.len() + 1);
        framed.push(1u8); // channel 1: packfile data
        framed.extend_from_slice(chunk);
        out.extend_from_slice(&pktline::encode_raw(&framed));
    }
    // A bare channel byte closes the side band, then a flush.
    out.extend_from_slice(&pktline::encode("\u{1}"));
    out.extend_from_slice(&pktline::flush());
    Ok(out)
}

/// One `ACK <id> common` line per held id that is a commit the project holds, repeats
/// included, as git does. An id that is not 40 hexadecimal characters, or names an object
/// that is not a commit, is treated as not stated (declared differences of the spec); an
/// unknown id is not acknowledged. Returns the commits acknowledged, in order.
fn acknowledge(repo: &Repository, haves: &[String], out: &mut Vec<u8>) -> Result<Vec<Oid>> {
    let mut commons = Vec::new();
    for have in haves {
        if have.len() != 40 || !have.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let Ok(oid) = Oid::from_str(have) else { continue };
        match repo.find_object(oid, None) {
            Ok(object) if object.kind() == Some(ObjectType::Commit) => {
                out.extend_from_slice(&pktline::encode(&format!("ACK {oid} common\n")));
                commons.push(oid);
            }
            Ok(_) => {}
            // Only "not found" means the project does not hold the id; a corrupt object or
            // an I/O error must not silently drop acknowledgements.
            Err(e) if e.code() == git2::ErrorCode::NotFound => {}
            Err(e) => return Err(ToolError::internal(format!("object lookup failed: {e}"))),
        }
    }
    Ok(commons)
}

/// The commit a wanted id leads to: itself for a commit, the tagged commit for an annotated
/// tag. `None` for a tree or a file version.
fn peel_commit(repo: &Repository, oid: Oid) -> Option<Oid> {
    repo.find_object(oid, None).ok()?.peel_to_commit().ok().map(|c| c.id())
}

/// The commits git treats as the edge of what the client holds: the parents of the commits
/// that will be sent which the client already has, and the wanted commits it already has.
/// libgit2 marks the contents of the commits it is told to hide as already known, so hiding
/// exactly these (not every held commit) selects the same objects as git does.
fn edge_commits(repo: &Repository, wants: &[Oid], commons: &[Oid]) -> Result<Vec<Oid>> {
    let mut walk =
        repo.revwalk().map_err(|e| ToolError::internal(format!("revwalk failed: {e}")))?;
    for want in wants {
        walk.push(*want).map_err(|e| ToolError::internal(format!("revwalk push failed: {e}")))?;
    }
    for common in commons {
        walk.hide(*common).map_err(|e| ToolError::internal(format!("revwalk hide failed: {e}")))?;
    }
    let mut included = HashSet::new();
    for oid in walk {
        included.insert(oid.map_err(|e| ToolError::internal(format!("revwalk failed: {e}")))?);
    }
    let mut seen = HashSet::new();
    let mut edges = Vec::new();
    for want in wants {
        if !included.contains(want) && seen.insert(*want) {
            edges.push(*want);
        }
    }
    for oid in &included {
        let commit = repo
            .find_commit(*oid)
            .map_err(|e| ToolError::internal(format!("find commit failed: {e}")))?;
        for parent in commit.parent_ids() {
            if !included.contains(&parent) && seen.insert(parent) {
                edges.push(parent);
            }
        }
    }
    Ok(edges)
}

/// Build the packfile for the wanted commits, minus what the client holds.
///
/// The pack must contain the wanted objects AND everything reachable from them, parents
/// included. `insert_recursive` only adds an object plus what it directly references (for a
/// commit: its tree, recursively), NOT its ancestry, so a repository with more than one
/// commit produced a pack that made the client fail with "Failed to traverse parents". A
/// revwalk over the tips, fed to `insert_walk`, is the API that yields a complete pack.
fn build_pack(repo: &Repository, wants: &[String], commons: &[Oid]) -> Result<Vec<u8>> {
    let mut buf = git2::Buf::new();
    {
        let mut pb = repo
            .packbuilder()
            .map_err(|e| ToolError::internal(format!("packbuilder failed: {e}")))?;
        let mut commit_wants = Vec::new();
        let mut walk =
            repo.revwalk().map_err(|e| ToolError::internal(format!("revwalk failed: {e}")))?;
        for want in wants {
            // Unknown or unreachable wants are skipped, like the reference does.
            let Ok(oid) = Oid::from_str(want) else { continue };
            if let Some(commit) = peel_commit(repo, oid) {
                // An annotated tag leads to its commit; the tag object itself is a later story.
                walk.push(commit)
                    .map_err(|e| ToolError::internal(format!("revwalk push failed: {e}")))?;
                commit_wants.push(commit);
            } else {
                // A bare tree or a file version can be wanted directly.
                let _ = pb.insert_recursive(oid, None);
            }
        }
        if !commit_wants.is_empty() {
            if !commons.is_empty() {
                for edge in edge_commits(repo, &commit_wants, commons)? {
                    walk.hide(edge)
                        .map_err(|e| ToolError::internal(format!("revwalk hide failed: {e}")))?;
                }
            }
            pb.insert_walk(&mut walk)
                .map_err(|e| ToolError::internal(format!("pack walk failed: {e}")))?;
        }
        pb.write_buf(&mut buf)
            .map_err(|e| ToolError::internal(format!("pack write failed: {e}")))?;
    }
    let pack = buf.to_vec();
    if pack.is_empty() {
        return Ok(empty_pack());
    }
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Two commits and an annotated tag on the second; the client holds the first.
    fn repo_with_tag() -> (tempfile::TempDir, Repository, Oid, Oid, Oid) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init_bare(dir.path()).unwrap();
        let sig =
            git2::Signature::new("t", "t@x.test", &git2::Time::new(1_700_000_000, 0)).unwrap();
        let mut parent: Option<Oid> = None;
        let mut ids = Vec::new();
        for name in ["a.txt", "b.txt"] {
            let blob = repo.blob(name.as_bytes()).unwrap();
            let mut tb = repo.treebuilder(None).unwrap();
            tb.insert(name, blob, 0o100644).unwrap();
            let tree = repo.find_tree(tb.write().unwrap()).unwrap();
            let parents: Vec<git2::Commit> =
                parent.iter().map(|p| repo.find_commit(*p).unwrap()).collect();
            let refs: Vec<&git2::Commit> = parents.iter().collect();
            let id = repo.commit(None, &sig, &sig, name, &tree, &refs).unwrap();
            ids.push(id);
            parent = Some(id);
        }
        let tag = {
            let target = repo.find_object(ids[1], None).unwrap();
            repo.tag("v1", &target, &sig, "first release", false).unwrap()
        };
        (dir, repo, ids[0], ids[1], tag)
    }

    /// Wanting an annotated tag while holding a commit must not hide the tagged commit: the
    /// edge computation works on the peeled commit, not on the tag id.
    #[test]
    fn a_wanted_tag_leads_to_its_commit_beyond_the_held_one() {
        let (_d, repo, held, tagged, tag) = repo_with_tag();
        let pack = build_pack(&repo, &[tag.to_string()], &[held]).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let client = Repository::init_bare(scratch.path()).unwrap();
        let odb = client.odb().unwrap();
        let mut w = odb.packwriter().unwrap();
        w.write_all(&pack).unwrap();
        w.commit().unwrap();
        assert!(odb.exists(tagged), "the commit the tag points at is sent");
        assert!(!odb.exists(held), "the held commit is not");
    }
}
