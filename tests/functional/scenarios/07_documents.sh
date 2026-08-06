#!/usr/bin/env bash
# Scénario 07 — fs.extract_text sur TXT, HTML, CSV
# Tools couverts: fs.extract_text, fs.write_docx (native Rust, sans pandoc)

suite "07 · Extraction de documents"

PROJ="ft-docs-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

# TXT — pas de companion
run_agent "Dans $PROJ, écris /note.txt avec : 'Ceci est une note simple.'" >/dev/null
OUT=$(run_agent "Extrais le texte de /note.txt dans $PROJ.")
assert_contains "extract_text txt retourne le texte" "$OUT" "note simple"

# HTML — companion .md créé
run_agent "Dans $PROJ, écris /page.html avec :
<html><body><h1>Mon Titre</h1><p>Un paragraphe <strong>important</strong>.</p></body></html>" >/dev/null
OUT=$(run_agent "Extrais le texte de /page.html dans $PROJ et indique-moi le chemin du companion markdown.")
assert_contains "extract_text html retourne titre" "$OUT" "Mon Titre\|mon titre"
assert_contains "companion md créé" "$OUT" "page.md\|\.md"

OUT=$(run_agent "Est-ce que /page.md existe dans $PROJ ?")
assert_contains "companion existe sur le volume" "$OUT" "oui\|yes\|true\|exist"

# CSV — tableau markdown
run_agent "Dans $PROJ, écris /data.csv avec :
name,score,grade
Alice,95,A
Bob,82,B
Carol,78,C" >/dev/null
OUT=$(run_agent "Extrais le texte de /data.csv dans $PROJ.")
assert_contains "extract csv retourne tableau" "$OUT" "Alice\|alice"
assert_contains "extract csv retourne score" "$OUT" "95\|score"

# DOCX natif (fs.write_docx — Rust pur, pas pandoc)
run_agent "Dans $PROJ, écris /report.md avec :
# Rapport de test
## Section 1
Contenu de la section 1.
| Col A | Col B |
|---|---|
| 1 | 2 |" >/dev/null
OUT=$(run_agent "Dans $PROJ, génère un fichier Word /report.docx à partir de /report.md (utilise fs.write_docx).")
assert_contains "write_docx créé" "$OUT" "report.docx\|docx\|word"

OUT=$(run_agent "Est-ce que /report.docx existe dans $PROJ ?")
assert_contains "docx présent sur le volume" "$OUT" "oui\|yes\|true\|exist"

run_agent "Supprime le projet $PROJ." >/dev/null
