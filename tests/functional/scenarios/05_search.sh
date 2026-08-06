#!/usr/bin/env bash
# Scénario 05 — fs.glob / fs.grep / fs.find_definition / fs.find_references
# Tools couverts: fs.glob, fs.grep (3 modes), fs.find_definition, fs.find_references

suite "05 · Recherche — glob, grep, find"

PROJ="ft-search-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

run_agent "Dans $PROJ, écris ces fichiers :
- /src/main.rs avec : fn main() { println!(\"hello world\"); }
- /src/utils.rs avec : pub fn add(a: i32, b: i32) -> i32 { a + b }  pub fn sub(a: i32, b: i32) -> i32 { a - b }
- /src/tests.rs avec : fn test_add() { assert_eq!(add(1,2), 3); }  fn test_sub() { assert_eq!(sub(5,3), 2); }
- /docs/guide.md avec : # Guide  See utils.rs for the math functions." >/dev/null

OUT=$(run_agent "Dans $PROJ, trouve tous les fichiers .rs avec glob.")
assert_contains "glob trouve main.rs" "$OUT" "main.rs"
assert_contains "glob trouve utils.rs" "$OUT" "utils.rs"
assert_not_contains "glob exclut .md" "$OUT" "guide.md"

OUT=$(run_agent "Dans $PROJ, cherche la chaîne 'fn' dans tous les fichiers — mode count, combien d'occurrences par fichier ?")
assert_contains "grep count sur main.rs" "$OUT" "main.rs"
assert_contains "grep count sur utils.rs" "$OUT" "utils.rs"

OUT=$(run_agent "Dans $PROJ, grep 'println' en mode content avec 1 ligne de contexte.")
assert_contains "grep content trouve println" "$OUT" "println"
assert_contains "grep content inclut contexte" "$OUT" "main\|fn"

OUT=$(run_agent "Dans $PROJ, quels fichiers contiennent le mot 'assert' — mode files seulement ?")
assert_contains "grep files trouve tests.rs" "$OUT" "tests.rs"
assert_not_contains "grep files exclut utils.rs" "$OUT" "utils.rs"

OUT=$(run_agent "Dans $PROJ, trouve la définition de la fonction 'add' dans /src.")
assert_contains "find_definition localise add" "$OUT" "utils.rs"
assert_contains "find_definition indique la ligne" "$OUT" "1\|ligne\|line"

OUT=$(run_agent "Dans $PROJ, trouve toutes les références à 'add' dans /src.")
assert_contains "find_references trouve tests.rs" "$OUT" "tests.rs"

run_agent "Supprime le projet $PROJ." >/dev/null
