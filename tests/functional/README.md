# Tests fonctionnels — agent CLI

Chaque scénario pilote le vrai binaire `agent` via stdin et valide les sorties.
Aucune interaction manuelle requise.

## Démarrage rapide

```bash
# 1. Démarrer le serveur
./run.sh

# 2. Exporter la clé LLM
export IBM_ICA_MODEL_KEY=...       # ou la variable nommée dans config/agent_test.yaml

# 3. S'assurer qu'un token agent existe
mkdir -p .agent_keys
./target/release/mcp-fs token admin@example.com --key .keys/jwt.key > .agent_keys/admin

# 4. Lancer tous les tests
./tests/functional/run_all.sh

# 5. Ou un seul scénario
./tests/functional/run_all.sh crud
./tests/functional/run_all.sh --user seb.morand@gmail.com search
```

## Scénarios

| # | Fichier | Tools couverts |
|---|---|---|
| 01 | `01_admin_lifecycle.sh` | `admin.create_project`, `admin.list_projects`, `admin.add_member`, `admin.list_members`, `admin.remove_member`, `admin.delete_project` |
| 02 | `02_fs_crud.sh` | `fs.write`, `fs.read`, `fs.stat`, `fs.exists`, `fs.create_empty`, `fs.delete` |
| 03 | `03_fs_edit.sh` | `fs.edit`, `fs.overwrite`, `fs.insert_at_line`, `fs.append` |
| 04 | `04_fs_tree.sh` | `fs.copy`, `fs.move`, `fs.delete`, `fs.list`, `fs.tree`, `fs.mkdir` |
| 05 | `05_search.sh` | `fs.glob`, `fs.grep` (3 modes), `fs.find_definition`, `fs.find_references` |
| 06 | `06_read_advanced.sh` | `fs.read_lines`, `fs.read_many`, `fs.read_bytes`, `fs.count_lines`, `fs.hash` |
| 07 | `07_documents.sh` | `fs.extract_text` (TXT/HTML/CSV), `fs.write_docx` |
| 08 | `08_database.sh` | `db.query`, `db.schema`, `db.sample`, `sqlite.create_table`, `sqlite.insert`, `sqlite.query`, `sqlite.list_tables` |
| 09 | `09_git.sh` | `git.init`, `git.log`, `git.refs`, `git.status`, `git.diff` — skip si git désactivé |
| 10 | `10_security.sh` | read_guard, no-clobber, soft-delete, isolation projets |
| 11 | `11_pandoc.sh` | `doc.to_docx`, `doc.to_pptx` — skip si pandoc absent |
| 12 | `12_editor.sh` | `doc.open_editor`, `doc.close_editor`, `doc.list_editors` — skip si --doc absent |
| 13 | `13_workflow.sh` | Session multi-tours : `admin.*` + `fs.*` + `db.query` enchaînés |

## Comment ça marche

Le runner (`run_all.sh`) :
1. Démarre le serveur si nécessaire (le stoppe à la fin)
2. Compile l'agent
3. Source chaque `scenarios/NN_*.sh` dans l'ordre
4. Chaque scénario appelle `run_agent "prompt..."` qui pipe le texte dans le binaire agent
5. `assert_contains` / `assert_not_contains` valident les sorties avec `grep -qi`
6. Bilan final : total / pass / fail / skip

## Variables d'environnement

| Variable | Valeur par défaut | Description |
|---|---|---|
| `IBM_ICA_MODEL_KEY` | — | Clé API LLM (obligatoire) |
| `VERBOSE` | `0` | Mettre à `1` pour voir les sorties brutes en cas d'échec |

## Ajouter un scénario

Créer `tests/functional/scenarios/NN_nom.sh` avec :

```bash
suite "NN · Nom du scénario"

PROJ="ft-nom-$$"
run_agent "Crée un projet $PROJ..." >/dev/null

OUT=$(run_agent "Prompt de test...")
assert_contains "description" "$OUT" "pattern regex attendu"

run_agent "Supprime le projet $PROJ." >/dev/null
```

Conventions :
- Nommer le projet avec un suffixe `$$` (PID) pour éviter les collisions entre runs parallèles
- Toujours supprimer le projet en fin de scénario
- Utiliser `skip` pour les features optionnelles (git, pandoc, --doc)
- `assert_contains` accepte des regex étendues (`\|` = OR, `.*` = wildcard)
