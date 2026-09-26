#!/usr/bin/env python3
"""Cross-spec audit: citation verification, ID uniqueness, and coverage gap analysis.

Phase 6 of the /spec-feat contract asks an auditor to open every file:LINE a spec
cites and to recount every count it asserts. This does the mechanical half:

  1. every cited path resolves to a real file
  2. every cited line number exists in that file
  3. IDs are unique across the whole corpus
  4. every source module is claimed by at least one spec (coverage gap analysis)

Findings needing a human judgement (does the cited line SAY what the spec claims)
are left to the reading pass; this narrows where that pass must look.

Usage: python3 specs/audit.py
"""
import os
import re
import sys
from collections import defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPECS = os.path.join(ROOT, 'specs')

# A citation is path.ext:LINE or path.ext:LINE-LINE, optionally inside backticks.
CITE = re.compile(r'([A-Za-z0-9_./-]+\.(?:rs|toml|txt|json|md|py|sh|yml|yaml)):(\d+)(?:-(\d+))?')

# Paths a spec cites relative to a crate root rather than the repo root.
SEARCH_PREFIXES = ['', 'crates/mcp-fs/src/', 'crates/mcp-fs/', 'crates/agent/src/']


_BY_BASENAME = None


def _index_basenames():
    """Specs cite a bare `meta.rs` once the full path is established nearby.
    Resolve those the way a reader does, and only when the name is unambiguous.
    """
    global _BY_BASENAME
    if _BY_BASENAME is not None:
        return _BY_BASENAME
    index = defaultdict(list)
    for base in ['crates', 'tests', 'scripts', 'config']:
        for dirpath, _, names in os.walk(os.path.join(ROOT, base)):
            if '/target/' in dirpath:
                continue
            for n in names:
                index[n].append(os.path.join(dirpath, n))
    _BY_BASENAME = index
    return index


def resolve(path):
    if path in _RESOLVED:
        return _RESOLVED[path]
    _RESOLVED[path] = _resolve_uncached(path)
    return _RESOLVED[path]


def _resolve_uncached(path):
    for prefix in SEARCH_PREFIXES:
        candidate = os.path.join(ROOT, prefix + path)
        if os.path.isfile(candidate):
            return candidate
    # Bare or partial name: accept only an unambiguous match.
    hits = _index_basenames().get(os.path.basename(path), [])
    if len(path.split('/')) > 1:
        tail = '/' + path
        hits = [h for h in hits if h.endswith(tail)]
    if len(hits) == 1:
        return hits[0]
    return None


_LINES = {}
_RESOLVED = {}


def line_count(path):
    if path not in _LINES:
        with open(path, 'rb') as fh:
            _LINES[path] = sum(1 for _ in fh)
    return _LINES[path]


def spec_files():
    # Each spec lives at specs/SPEC-NNNN_<timestamp>-<slug>/spec.md (protocol layout).
    # Archived specs are excluded: they are out of active scope, same as the original
    # flat-file listing never re-scanned specs it had already closed out.
    out = []
    for name in sorted(os.listdir(SPECS)):
        spec_path = os.path.join(SPECS, name, 'spec.md')
        if os.path.isfile(spec_path):
            out.append(spec_path)
    return out


def audit_citations(files):
    bad_path, bad_line, ok = [], [], 0
    counts = defaultdict(int)
    for path in files:
        name = os.path.basename(os.path.dirname(path))
        text = open(path).read()
        for m in CITE.finditer(text):
            cited, start, end = m.group(1), int(m.group(2)), m.group(3)
            # Skip self-references to spec files.
            if cited.endswith('.md') and 'specs/' not in cited and '-' in cited:
                continue
            resolved = resolve(cited)
            counts[name] += 1
            if resolved is None:
                bad_path.append((name, cited))
                continue
            n = line_count(resolved)
            last = int(end) if end else start
            if last > n or start < 1:
                bad_line.append((name, f'{cited}:{m.group(2)}', f'file has {n} lines'))
            else:
                ok += 1
    return bad_path, bad_line, ok, counts


def audit_ids(files):
    owner = {}
    dupes = []
    for path in files:
        name = os.path.basename(os.path.dirname(path))
        text = open(path).read()
        ids = set()
        ids |= set(re.findall(r'^### (SC-\d{3})', text, re.M))
        ids |= set(re.findall(r'^#### (FR-\d{3})', text, re.M))
        ids |= set(re.findall(r'^#### (E2E-\d{3}):', text, re.M))
        ids |= set(re.findall(r'^- \*\*(DEC-\d{3}):', text, re.M))
        for i in ids:
            if i in owner and owner[i] != name:
                dupes.append((i, owner[i], name))
            owner[i] = name
    return dupes


def audit_coverage(files):
    """Every source module must be claimed by at least one spec."""
    claimed = set()
    for path in files:
        text = open(path).read()
        for m in CITE.finditer(text):
            resolved = resolve(m.group(1))
            if resolved:
                claimed.add(os.path.relpath(resolved, ROOT))
        # A spec also claims a module by naming it without a line number,
        # including the brace form `tools/{read,write}.rs`.
        for m in re.findall(r'`([A-Za-z0-9_./-]*\{[a-z_,]+\}[A-Za-z0-9_./-]*\.rs)`', text):
            head, rest = m.split('{', 1)
            names, tail = rest.split('}', 1)
            for n in names.split(','):
                r = resolve(head + n + tail)
                if r:
                    claimed.add(os.path.relpath(r, ROOT))
        for m in re.findall(r'`([A-Za-z0-9_./-]+\.rs)`', text):
            r = resolve(m)
            if r:
                claimed.add(os.path.relpath(r, ROOT))
        for m in set(re.findall(r'`([a-z_]+(?:/[a-z_]+)*/)`', text)):
            d = os.path.join(ROOT, 'crates/mcp-fs/src', m)
            if os.path.isdir(d):
                for n in os.listdir(d):
                    if n.endswith('.rs'):
                        claimed.add(os.path.relpath(os.path.join(d, n), ROOT))

    sources = []
    for base in ['crates/mcp-fs/src', 'crates/agent/src']:
        for dirpath, _, names in os.walk(os.path.join(ROOT, base)):
            for n in names:
                if n.endswith('.rs'):
                    sources.append(os.path.relpath(os.path.join(dirpath, n), ROOT))
    return sorted(set(sources) - claimed), len(sources)


def main():
    files = spec_files()
    print(f'Auditing {len(files)} specifications\n')

    bad_path, bad_line, ok, per_spec = audit_citations(files)
    total = ok + len(bad_path) + len(bad_line)
    print(f'CITATIONS: {total} checked, {ok} resolve to a real file and a real line')
    for name, cited in bad_path:
        print(f'  BROKEN PATH   {name}: {cited} does not exist')
    for name, cited, why in bad_line:
        print(f'  BROKEN LINE   {name}: {cited} ({why})')
    if not bad_path and not bad_line:
        print('  no broken citations')
    print('  per spec:', ', '.join(f'{k.split("-")[-1][:-3]}={v}' for k, v in sorted(per_spec.items())))

    print()
    dupes = audit_ids(files)
    print(f'ID UNIQUENESS: {"no collisions" if not dupes else f"{len(dupes)} collisions"}')
    for i, a, b in dupes:
        print(f'  COLLISION {i} defined in both {a} and {b}')

    print()
    unclaimed, n_src = audit_coverage(files)
    print(f'COVERAGE: {n_src - len(unclaimed)}/{n_src} source modules claimed by a spec')
    for u in unclaimed:
        print(f'  UNCLAIMED {u}')

    return 1 if (bad_path or bad_line or dupes) else 0


if __name__ == '__main__':
    sys.exit(main())
