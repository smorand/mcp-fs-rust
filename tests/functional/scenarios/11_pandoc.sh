#!/usr/bin/env bash
# Scénario 11 — doc.to_docx / doc.to_pptx (pandoc)
# Tools couverts: doc.to_docx, doc.to_pptx
#
# Prérequis : pandoc installé + serveur démarré avec --doc

suite "11 · Pandoc — conversion DOCX et PPTX"

PROJ="ft-pandoc-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

# Vérifier si doc.to_docx est disponible
PROBE=$(run_agent "Liste tous les outils doc.* disponibles." 2>&1)
if echo "$PROBE" | grep -qi "to_docx\|to_pptx"; then
  HAS_PANDOC=1
else
  HAS_PANDOC=0
fi

if [[ $HAS_PANDOC -eq 0 ]]; then
  skip "doc.to_docx" "pandoc absent ou --doc non activé"
  skip "doc.to_pptx" "pandoc absent ou --doc non activé"
  run_agent "Supprime le projet $PROJ." >/dev/null
  return
fi

# ── Markdown → DOCX ──────────────────────────────────────────────────────────
run_agent "Dans $PROJ, écris /report.md avec :
# Rapport Trimestriel

## Résultats

Les résultats sont **excellents** cette année.

| Métrique | Q3 | Q4 |
|---|---|---|
| Revenu | 1.2M | 1.8M |
| Marge | 32% | 38% |" >/dev/null

OUT=$(run_agent "Dans $PROJ, convertis /report.md en /report.docx avec doc.to_docx.")
assert_contains "to_docx créé" "$OUT" "report.docx\|docx\|ok\|bytes"

OUT=$(run_agent "Est-ce que /report.docx existe dans $PROJ ?")
assert_contains "docx présent" "$OUT" "oui\|yes\|true\|exist"

# ── no-clobber ───────────────────────────────────────────────────────────────
OUT=$(run_agent "Dans $PROJ, essaie de reconvertir /report.md en /report.docx sans overwrite.")
assert_contains "no-clobber déclenché" "$OUT" "exist\|409\|already\|déjà\|erreur\|error"

# ── overwrite explicite ───────────────────────────────────────────────────────
OUT=$(run_agent "Dans $PROJ, reconvertis /report.md en /report.docx avec overwrite=true.")
assert_contains "overwrite accepté" "$OUT" "report.docx\|ok\|bytes"

# ── HTML → DOCX ──────────────────────────────────────────────────────────────
run_agent "Dans $PROJ, écris /page.html avec :
<html><body><h1>Titre HTML</h1><p>Un <em>paragraphe</em> converti.</p></body></html>" >/dev/null

OUT=$(run_agent "Dans $PROJ, convertis /page.html en /page.docx avec doc.to_docx.")
assert_contains "html to_docx créé" "$OUT" "page.docx\|ok\|bytes"

# ── Markdown → PPTX ──────────────────────────────────────────────────────────
run_agent "Dans $PROJ, écris /pres.md avec :
# Slide 1 : Introduction

Bienvenue dans cette présentation.

---

# Slide 2 : Objectifs

- Objectif A
- Objectif B
- Objectif C" >/dev/null

OUT=$(run_agent "Dans $PROJ, convertis /pres.md en /pres.pptx avec doc.to_pptx.")
assert_contains "to_pptx créé" "$OUT" "pres.pptx\|pptx\|ok\|bytes"

OUT=$(run_agent "Est-ce que /pres.pptx existe dans $PROJ ?")
assert_contains "pptx présent" "$OUT" "oui\|yes\|true\|exist"

# ── mauvaise extension ────────────────────────────────────────────────────────
OUT=$(run_agent "Dans $PROJ, essaie de convertir /report.md en /report.pdf avec doc.to_docx — que se passe-t-il ?")
assert_contains "mauvaise extension rejetée" "$OUT" "invalid\|erreur\|error\|extension\|pdf\|docx"

run_agent "Supprime le projet $PROJ." >/dev/null
