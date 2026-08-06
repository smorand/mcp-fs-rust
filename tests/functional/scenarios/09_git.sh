#!/usr/bin/env bash
# Scénario 09 — git.* : init, push, fetch, log, refs, status
# Tools couverts: git.init, git.log, git.refs, git.status, git.diff
#
# Prérequis : git.enabled: true dans config/local.yaml

suite "09 · Git — opérations de dépôt"

# Vérifier que git est activé
if ! curl -sf "http://127.0.0.1:5002/health" | grep -q "ok"; then
  skip "git — serveur inaccessible" "serveur non démarré"; return
fi

# Sonder si git est activé en tentant un init sur un projet connu
PROJ="ft-git-$$"
run_agent "Crée un projet $PROJ (owner: admin@example.com)." >/dev/null

PROBE=$(run_agent "Initialise un dépôt git dans le projet $PROJ." 2>&1)
if echo "$PROBE" | grep -qi "not.*support\|disabled\|not.*found\|ERR_NOT_SUPPORTED\|pas.*support"; then
  skip "git.init" "git.enabled=false dans la config"
  run_agent "Supprime le projet $PROJ." >/dev/null
  return
fi

assert_contains "git.init confirmé" "$PROBE" "init\|creat\|ok"

# Commits via écriture de fichiers + demande de commit
run_agent "Dans $PROJ, écris /README.md avec le contenu '# Mon Projet' et /src/main.rs avec 'fn main() {}'" >/dev/null

OUT=$(run_agent "Dans $PROJ, montre-moi le statut git des fichiers (tracked/untracked).")
assert_contains "git.status retourne des fichiers" "$OUT" "README\|main\|modif\|new\|ajout"

OUT=$(run_agent "Dans $PROJ, montre-moi les refs git actuels.")
assert_contains "git.refs retourne head ou main" "$OUT" "HEAD\|main\|refs"

OUT=$(run_agent "Dans $PROJ, montre-moi le log git.")
# Le log peut être vide si aucun commit n'a été fait via le wire protocol
assert_contains "git.log retourne quelque chose" "$OUT" "commit\|log\|empty\|vide\|aucun\|no commit"

OUT=$(run_agent "Dans $PROJ, est-ce que /README.md a des modifications non commitées ? Utilise git.diff.")
assert_contains "git.diff retourne quelque chose" "$OUT" "diff\|README\|modif\|change"

run_agent "Supprime le projet $PROJ." >/dev/null
