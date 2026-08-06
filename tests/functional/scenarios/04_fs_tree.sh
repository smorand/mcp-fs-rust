#!/usr/bin/env bash
# Scénario 04 — fs.copy / fs.move / fs.delete / fs.list / fs.tree / fs.mkdir
# Tools couverts: fs.copy, fs.move, fs.delete, fs.list, fs.tree, fs.mkdir

suite "04 · Filesystem — arborescence et déplacements"

PROJ="ft-tree-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

# Créer une arborescence
run_agent "Dans $PROJ, écris ces 3 fichiers :
- /src/main.rs avec le contenu 'fn main() {}'
- /src/lib.rs avec le contenu 'pub fn hello() {}'
- /docs/README.md avec le contenu '# Docs'" >/dev/null

OUT=$(run_agent "Liste le répertoire /src dans $PROJ.")
assert_contains "list voit main.rs" "$OUT" "main.rs"
assert_contains "list voit lib.rs" "$OUT" "lib.rs"

OUT=$(run_agent "Montre-moi l'arborescence complète du projet $PROJ.")
assert_contains "tree voit /docs" "$OUT" "docs"
assert_contains "tree voit /src" "$OUT" "src"

OUT=$(run_agent "Crée le répertoire /archive dans $PROJ.")
assert_contains "mkdir confirmé" "$OUT" "archive\|créé\|ok"

OUT=$(run_agent "Copie /src/main.rs vers /archive/main.rs.bak dans $PROJ.")
assert_contains "copy confirmé" "$OUT" "copié\|copi\|ok"

OUT=$(run_agent "Est-ce que /archive/main.rs.bak existe dans $PROJ ?")
assert_contains "copie présente" "$OUT" "oui\|yes\|true\|exist"

OUT=$(run_agent "Déplace /docs/README.md vers /archive/README.md dans $PROJ.")
assert_contains "move confirmé" "$OUT" "déplacé\|mov\|ok"

OUT=$(run_agent "Est-ce que /docs/README.md existe encore dans $PROJ ?")
assert_contains "source disparue" "$OUT" "non\|no\|false\|n'exist\|not"

OUT=$(run_agent "Supprime le répertoire /archive et tout son contenu dans $PROJ.")
assert_contains "delete recursive confirmé" "$OUT" "supprim\|delet"

OUT=$(run_agent "Montre-moi l'arborescence de $PROJ.")
assert_not_contains "archive disparu" "$OUT" "archive"

run_agent "Supprime le projet $PROJ." >/dev/null
