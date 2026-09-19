#!/usr/bin/env python3
"""Step 4.5 self-consistency gate for a spec file.

Runs the ten deterministic checks from the /spec-feat contract that can be
decided mechanically. Exits non-zero on any violation so it can gate a commit.

Usage: python3 specs/check.py specs/<file>.md [...]
"""
import re
import sys


def section(text, n, nxt):
    m = re.search(rf'^## {n}\..*?(?=^## {nxt}\.)', text, re.M | re.S)
    return m.group(0) if m else ''


def check(path):
    t = open(path).read()
    name = path.split('/')[-1]
    if '## 6. Functional Requirements' not in t:
        print(f'SKIP {name}: not a specification document')
        return 0
    bad = []
    s5 = section(t, 5, 6)
    s6 = section(t, 6, 7)
    s11 = section(t, 11, 12)
    s12 = section(t, 12, 13)

    # 1-2: ids defined and traced
    frs = sorted(set(re.findall(r'#### (FR-[\w-]*\d{3})', s6)))
    scs = sorted(set(re.findall(r'### (SC-\d{3})', s5)))
    for f in frs:
        if f not in s11:
            bad.append(f'check2: {f} defined but absent from the traceability matrix')
    for s in scs:
        if s not in s11:
            bad.append(f'check1: {s} defined but absent from the traceability matrix')
    for f in sorted(set(re.findall(r'FR-[\w-]*\d{3}', s11))):
        if f not in frs:
            bad.append(f'check2: {f} in the matrix but never defined in section 6')

    # 3: every test in the summary names a scenario and at least one FR
    rows = re.findall(r'^\| (E2E-[\w-]*\d{3}) \| \w+ \| [^|]+ \| ([^|]+) \| ([^|]+) \|', s12, re.M)
    for tid, sc, fr in rows:
        if not re.search(r'SC-\d{3}', sc):
            bad.append(f'check3: {tid} names no scenario')
        if not re.search(r'FR-[\w-]*\d{3}', fr):
            bad.append(f'check3: {tid} names no requirement')

    summary = {r[0] for r in rows}
    matrix = set(re.findall(r'E2E-[\w-]*\d{3}', s11))
    for x in sorted(matrix - summary):
        bad.append(f'check6: {x} in the matrix but missing from the test summary')
    for x in sorted(summary - matrix):
        bad.append(f'check6: {x} in the summary but never traced in the matrix')

    # every New test must be fully specified
    new = {r[0] for r in rows if re.search(rf'\| {re.escape(r[0])} \| New ', s12)}
    spec = set(re.findall(r'#### (E2E-[\w-]*\d{3}):', s12))
    for x in sorted(new - spec):
        bad.append(f'check3: {x} is New but has no full specification in 12.2')

    # 4: each scenario carries happy, failure and edge coverage
    for line in re.findall(r'^\| (SC-\d{3}[^|]*) \|([^\n]*)$', s11, re.M):
        cells = line[1].split('|')
        if len(cells) >= 4:
            for idx, label in ((1, 'happy'), (2, 'failure'), (3, 'edge')):
                if not re.search(r'E2E-', cells[idx]):
                    bad.append(f'check4: {line[0].strip()} has no {label} test')

    # 5: every FR carries at least three tests in the per-FR table
    cov = {}
    for fr, tests in re.findall(r'\| (FR-[\w-]*\d{3}) \| ([^|]+)', s11):
        ids = re.findall(r'E2E-[\w-]*\d{3}', tests)
        n = len(ids)
        for a, b in re.findall(r'E2E-(\d{3})\.\.E2E-(\d{3})', tests):
            n = max(n, int(b) - int(a) + 1)
        cov[fr] = max(cov.get(fr, 0), n)
    for f in frs:
        if cov.get(f, 0) < 3:
            bad.append(f'check5: {f} has {cov.get(f, 0)} tests, needs at least 3')

    # 6: failure must outnumber happy
    m = re.search(r'Happy:Failure ratio: 1:([\d.]+)', s12)
    if not m:
        bad.append('check6: no happy:failure ratio stated')
    elif float(m.group(1)) <= 1.0:
        bad.append(f'check6: failure:happy ratio {m.group(1)} is not greater than 1')

    # 7-8: modals and EARS tags
    for blk in re.split(r'(?=#### FR-)', s6)[1:]:
        fid = re.match(r'#### (FR-[\w-]*\d{3})', blk).group(1)
        # The Priority field legitimately carries "Should-have"/"Nice-to-have".
        prose = re.sub(r'^- \*\*Priority:\*\*.*$', '', blk, flags=re.M)
        for w in ('should', 'may', 'could', 'might', 'would'):
            if re.search(r'\b' + w + r'\b', prose, re.I):
                bad.append(f'check7: {fid} uses the forbidden modal "{w}"')
        if not re.match(r'#### FR-[\w-]*\d{3} \[EARS-(U|E|S|O|UB|X)\]', blk):
            bad.append(f'check8: {fid} carries no valid EARS tag')
    tags = re.findall(r'#### FR-[\w-]*\d{3} \[EARS-(\w+)\]', s6)
    if tags and tags.count('X') > len(tags) * 0.10:
        bad.append(f'check8: X-escape used {tags.count("X")} times, above 10% of {len(tags)}')

    # 9: glossary populated and referenced
    g = section(t, 16, 17)
    terms = re.findall(r'^\| \*\*(.+?)\*\*', g, re.M)
    if not terms:
        bad.append('check9: glossary is empty')
    body = t[t.index('## 4. User Personas'):t.index('## 10. Documentation')]
    for term in terms:
        if term.lower().rstrip('s') not in body.lower():
            bad.append(f'check9: glossary term "{term}" is never referenced in sections 4-9')

    if bad:
        print(f'FAIL {name} ({len(bad)} violations)')
        for b in bad:
            print(f'  - {b}')
    else:
        print(f'PASS {name}: {len(scs)} scenarios, {len(frs)} requirements, {len(summary)} tests')
    return len(bad)


if __name__ == '__main__':
    total = sum(check(p) for p in sys.argv[1:])
    sys.exit(1 if total else 0)
