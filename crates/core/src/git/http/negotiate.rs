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
    /// Held ids as written, in order, wherever they appear in the request.
    pub haves: Vec<String>,
    /// The end marker was present: the client now expects the objects.
    pub done: bool,
    /// A shallow, deepen or filter line was present: options the service does not offer.
    pub option_line: bool,
    /// The first wanted line asked for `side-band-64k`: the objects go out framed on channel 1.
    /// Without it they go out as they are, like git's own server sends them.
    pub side_band: bool,
}

/// Read the whole request: every line, whatever its order, to the end of the input. Flushes
/// and the other control packets only separate the sections a client writes (wants, haves) and
/// carry no meaning here, so a missing one changes nothing; the end marker is noted and the
/// lines after it are still read. A line cut by the end of the input is dropped.
pub fn parse(body: &[u8]) -> Request {
    let mut req = Request::default();
    let mut reader = PktReader::new(body);
    while !reader.remaining().is_empty() {
        let (line, kind) = reader.read_line();
        // A line cut by the end of the request is dropped: only complete lines are read.
        if reader.last_was_cut() {
            break;
        }
        let (PacketType::Data, Some(line)) = (kind, line) else { continue };
        if let Some(rest) = line.strip_prefix("want ") {
            // The first want carries the client's options after a space.
            let mut words = rest.split_whitespace();
            req.wants.push(words.next().unwrap_or("").to_string());
            if req.wants.len() == 1 {
                req.side_band = words.any(|o| o == "side-band-64k");
            }
        } else if let Some(rest) = line.strip_prefix("have ") {
            req.haves.push(rest.trim().to_string());
        } else if line == "done" {
            req.done = true;
        } else if line.starts_with("shallow ")
            || line.starts_with("deepen")
            || line.starts_with("filter ")
        {
            req.option_line = true;
        }
    }
    req
}

/// The whole response body of a `git-upload-pack` request.
pub fn answer(repo: &Repository, ref_values: &[String], req: &Request) -> Result<Vec<u8>> {
    if req.wants.is_empty() {
        return Ok(pktline::flush());
    }
    let mut out = Vec::new();
    // Only what a branch or tag reaches is served. When nothing wanted can be served the
    // answer is one NAK and no object, whatever the client holds (declared difference: git
    // would refuse the request).
    let wants = servable_wants(repo, ref_values, &req.wants)?;
    if wants.is_empty() {
        out.extend_from_slice(&pktline::encode("NAK\n"));
        return finish(repo, &[], &[], req.side_band, out);
    }
    let (haves, done) = (&req.haves, req.done);
    // A request that states no held id is a clone, or a client that does not negotiate: one
    // NAK and everything reachable, as before the negotiation existed. So is one that asks
    // for an option the service does not offer (shallow, deepen, filter): honouring the held
    // ids without the option would send a different answer from the one it expects.
    if haves.is_empty() || req.option_line {
        out.extend_from_slice(&pktline::encode("NAK\n"));
        return finish(repo, &wants, &[], req.side_band, out);
    }
    let (commons, got_unknown) = acknowledge(repo, haves, &wants, &mut out)?;
    if !done {
        // A round that does not end the negotiation: acknowledgements only. Like git, say
        // the client has enough when it stated no id the project lacks and every wanted
        // commit leads to a commit it holds.
        if let Some(last) = commons.last()
            && !got_unknown
            && every_want_reaches(repo, &wants, &commons)?
        {
            out.extend_from_slice(&pktline::encode(&format!("ACK {last} ready\n")));
        }
        out.extend_from_slice(&pktline::encode("NAK\n"));
        return Ok(out);
    }
    match commons.last() {
        Some(last) => out.extend_from_slice(&pktline::encode(&format!("ACK {last}\n"))),
        None => out.extend_from_slice(&pktline::encode("NAK\n")),
    }
    finish(repo, &wants, &commons, req.side_band, out)
}

/// Append the objects the client lacks: framed on side-band channel 1 with the closing bytes
/// when the client asked for the side band, as they are otherwise.
fn finish(
    repo: &Repository,
    wants: &[String],
    commons: &[Oid],
    side_band: bool,
    mut out: Vec<u8>,
) -> Result<Vec<u8>> {
    let pack = build_pack(repo, wants, commons)?;
    if !side_band {
        // The client did not ask for the side band: the objects follow the acknowledgement
        // lines as they are, with no framing and nothing after them.
        out.extend_from_slice(&pack);
        return Ok(out);
    }
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

/// The wanted ids a branch or tag reaches, in order. An id that is not 40 hexadecimal
/// characters, that the project does not hold, or that no branch or tag reaches is dropped.
fn servable_wants(
    repo: &Repository,
    ref_values: &[String],
    wants: &[String],
) -> Result<Vec<String>> {
    // The commits the refs lead to (a tag is followed to its commit), built on first need: a
    // fetch of a branch tip is answered from `ref_values` alone.
    let mut tips: Option<Vec<Oid>> = None;
    let mut out = Vec::new();
    for want in wants {
        if want.len() != 40 || !want.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let Ok(oid) = Oid::from_str(want) else { continue };
        let object = match repo.find_object(oid, None) {
            Ok(o) => o,
            Err(e) if e.code() == git2::ErrorCode::NotFound => continue,
            Err(e) => return Err(ToolError::internal(format!("object lookup failed: {e}"))),
        };
        let reachable = if ref_values.iter().any(|v| v.eq_ignore_ascii_case(want)) {
            true
        } else {
            let tips = tips.get_or_insert_with(|| {
                ref_values
                    .iter()
                    .filter_map(|v| Oid::from_str(v).ok())
                    .filter_map(|o| peel_commit(repo, o))
                    .collect()
            });
            match object.kind() {
                Some(ObjectType::Commit) => commit_reachable(repo, tips, oid)?,
                Some(ObjectType::Tree | ObjectType::Blob) => content_reachable(repo, tips, oid)?,
                // An inner tag of a chain is not served unless a ref names it.
                _ => false,
            }
        };
        if reachable {
            out.push(want.clone());
        }
    }
    Ok(out)
}

/// Whether `oid` is a ref tip or in the history of one.
fn commit_reachable(repo: &Repository, tips: &[Oid], oid: Oid) -> Result<bool> {
    for tip in tips {
        if *tip == oid
            || repo
                .graph_descendant_of(*tip, oid)
                .map_err(|e| ToolError::internal(format!("ancestry check failed: {e}")))?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether a folder listing or a file version belongs to a commit some ref reaches. Rare (a
/// client wants such an object directly), so a walk over the history of every tip is fine; a
/// folder listing already seen is not entered twice.
fn content_reachable(repo: &Repository, tips: &[Oid], target: Oid) -> Result<bool> {
    let err = |e: git2::Error| ToolError::internal(format!("reachability walk failed: {e}"));
    let mut walk = repo.revwalk().map_err(err)?;
    for tip in tips {
        walk.push(*tip).map_err(err)?;
    }
    let mut seen = HashSet::new();
    for commit in walk {
        let tree = repo.find_commit(commit.map_err(err)?).and_then(|c| c.tree()).map_err(err)?;
        let mut stack = vec![tree.id()];
        while let Some(id) = stack.pop() {
            if id == target {
                return Ok(true);
            }
            if !seen.insert(id) {
                continue;
            }
            let tree = repo.find_tree(id).map_err(err)?;
            for entry in tree.iter() {
                match entry.kind() {
                    Some(ObjectType::Tree) => stack.push(entry.id()),
                    _ if entry.id() == target => return Ok(true),
                    _ => {}
                }
            }
        }
    }
    Ok(false)
}

/// One `ACK <id> common` line per held id that is a commit the project holds, repeats
/// included, as git does. An id that is not 40 hexadecimal characters, or names an object
/// that is not a commit, is treated as not stated (declared differences of the spec); an
/// unknown id is not acknowledged. Returns the commits acknowledged, in order, and whether a
/// well formed id the project does not hold was stated (git then withholds `ready`).
fn acknowledge(
    repo: &Repository,
    haves: &[String],
    wants: &[String],
    out: &mut Vec<u8>,
) -> Result<(Vec<Oid>, bool)> {
    let mut commons = Vec::new();
    let mut got_unknown = false;
    // The verdict of `every_want_reaches` only changes when a commit joins `commons`.
    let mut reaches: Option<(usize, bool)> = None;
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
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                got_unknown = true;
                // git cuts in as soon as it knows enough: an id it lacks, met after held
                // commits that already satisfy every want, is answered with `ready`
                // carrying that id.
                if !commons.is_empty() {
                    let enough = match reaches {
                        Some((n, verdict)) if n == commons.len() => verdict,
                        _ => {
                            let verdict = every_want_reaches(repo, wants, &commons)?;
                            reaches = Some((commons.len(), verdict));
                            verdict
                        }
                    };
                    if enough {
                        out.extend_from_slice(&pktline::encode(&format!("ACK {oid} ready\n")));
                    }
                }
            }
            Err(e) => return Err(ToolError::internal(format!("object lookup failed: {e}"))),
        }
    }
    Ok((commons, got_unknown))
}

/// Whether every wanted commit is a held commit or has one in its history. A want that leads
/// to no commit (a tree or a file version) is skipped, as git does.
fn every_want_reaches(repo: &Repository, wants: &[String], commons: &[Oid]) -> Result<bool> {
    for want in wants {
        let Some(commit) = Oid::from_str(want).ok().and_then(|o| peel_commit(repo, o)) else {
            continue;
        };
        let mut reaches = false;
        for common in commons {
            if commit == *common
                || repo
                    .graph_descendant_of(commit, *common)
                    .map_err(|e| ToolError::internal(format!("ancestry check failed: {e}")))?
            {
                reaches = true;
                break;
            }
        }
        if !reaches {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Add the tag objects between a wanted id and its commit (a tag may point at a tag).
fn insert_tag_chain(repo: &Repository, pb: &mut git2::PackBuilder<'_>, mut oid: Oid) -> Result<()> {
    while let Ok(tag) = repo.find_tag(oid) {
        pb.insert_object(oid, None)
            .map_err(|e| ToolError::internal(format!("pack insert failed: {e}")))?;
        oid = tag.target_id();
    }
    Ok(())
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
                // An annotated tag is sent as an object of its own, then its commit is
                // handled like a wanted commit (a held one sends nothing more).
                insert_tag_chain(repo, &mut pb, oid)?;
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

    fn pkt(line: &str) -> Vec<u8> {
        format!("{:04x}{line}", line.len() + 4).into_bytes()
    }

    /// Wanting an annotated tag while holding a commit must not hide the tagged commit: the
    /// edge computation works on the peeled commit, not on the tag id.
    /// The side band choice comes from the options of the first wanted line only.
    #[test]
    fn the_side_band_is_asked_for_on_the_first_wanted_line() {
        let id = "1".repeat(40);
        let with = parse(&pkt(&format!("want {id} multi_ack_detailed side-band-64k ofs-delta\n")));
        assert!(with.side_band);
        let without = parse(&pkt(&format!("want {id} multi_ack_detailed ofs-delta\n")));
        assert!(!without.side_band);
        let mut second = pkt(&format!("want {id} multi_ack_detailed\n"));
        second.extend(pkt(&format!("want {id} side-band-64k\n")));
        assert!(!parse(&second).side_band, "only the first wanted line carries options");
    }

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
