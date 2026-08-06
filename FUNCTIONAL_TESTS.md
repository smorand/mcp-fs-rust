# Tests fonctionnels mcp-fs

Guide de validation manuelle. Chaque scénario est indépendant, déroulable en 5 à 15
minutes avec `curl` et un terminal. Ils couvrent les chemins critiques que les tests
unitaires ne peuvent pas tester bout-en-bout: démarrage réel, JWT live, REST + MCP en
parallèle, éditeur HTML dans le navigateur.

---

## Prérequis

```bash
# 1. Démarrer le serveur (génère les clés et bootstrappe config/local.yaml si absent)
./run.sh

# 2. Dans un second terminal — toutes les commandes ci-dessous l'utilisent
export TOKEN=$(./target/release/mcp-fs token you@example.com --key .keys/jwt.key)
export MCP=http://127.0.0.1:5002
export H="Content-Type: application/json"
export A="Accept: application/json, text/event-stream"
export AUTH="X-Forwarded-Authorization: Bearer $TOKEN"

# Helper : appeler un outil MCP et afficher le résultat
mcp() {
  local tool=$1; local args=${2:-{}}
  curl -sf -X POST "$MCP/mcp" \
    -H "$H" -H "$A" -H "$AUTH" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",
         \"params\":{\"name\":\"$tool\",\"arguments\":$args}}" \
  | grep -o '"text":"[^"]*"' | sed 's/"text":"//;s/"$//' \
  | python3 -c "import sys,json; print(json.dumps(json.loads(sys.stdin.read()),indent=2))" \
  2>/dev/null || echo "(raw output above)"
}
```

---

## Scénario 1 — Démarrage et santé

**But :** vérifier que le serveur répond correctement, que les routes publiques sont
accessibles sans token, et que les routes protégées exigent un token valide.

```bash
# 1a. Health sans token
curl -sf $MCP/health
# Attendu : {"status":"ok","version":"..."}

# 1b. MCP sans token
curl -sf -X POST $MCP/mcp \
  -H "$H" -H "$A" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
# Attendu : 401 avec body {"code":"ERR_UNAUTHORIZED",...}

# 1c. MCP avec token valide
curl -sf -X POST $MCP/mcp \
  -H "$H" -H "$A" -H "$AUTH" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | python3 -m json.tool | head -30
# Attendu : SSE ou JSON avec une liste de tools dont "admin.list_projects"

# 1d. Token expiré (TTL = -1s)
BAD=$(./target/release/mcp-fs token you@example.com --key .keys/jwt.key --ttl -1)
curl -sf -X POST $MCP/mcp \
  -H "$H" -H "$A" -H "X-Forwarded-Authorization: Bearer $BAD" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
# Attendu : 401

# 1e. Token forgé (signature invalide)
curl -sf -X POST $MCP/mcp \
  -H "$H" -H "$A" -H "X-Forwarded-Authorization: Bearer eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJoYWNrZXIifQ.invalidsig" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
# Attendu : 401
```

**Critères de succès :** 1a vert sans auth, 1b-1e tous 401, 1c liste non vide.

---

## Scénario 2 — Cycle de vie projet (admin)

**But :** créer un projet, gérer les membres, supprimer.

```bash
# 2a. Créer un projet (nécessite d'être admin — vérifie config/local.yaml admins)
mcp admin.create_project '{"project_id":"demo","owner":"you@example.com"}'
# Attendu : {"project_id":"demo","owner":"you@example.com"}

# 2b. Lister les projets
mcp admin.list_projects '{}'
# Attendu : liste contenant "demo"

# 2c. Ajouter un membre
mcp admin.add_member '{"project_id":"demo","person":"alice@example.com"}'
# Attendu : {"project_id":"demo","person":"alice@example.com","added":true}

# 2d. Lister les membres
mcp admin.list_members '{"project_id":"demo"}'
# Attendu : vous + alice

# 2e. Supprimer le membre
mcp admin.remove_member '{"project_id":"demo","person":"alice@example.com"}'
# Attendu : {"removed":true}

# 2f. Supprimer le projet
mcp admin.delete_project '{"project_id":"demo"}'
# Attendu : {"deleted":true}

# 2g. Vérifier qu'il n'existe plus
mcp admin.list_projects '{}'
# Attendu : "demo" absent
```

**Critères de succès :** chaque étape retourne le shape attendu, le projet est absent en 2g.

---

## Scénario 3 — CRUD fichiers (MCP)

**But :** dérouler le cycle complet de fichiers via les tools MCP.

```bash
# Créer le projet de travail
mcp admin.create_project '{"project_id":"ws","owner":"you@example.com"}'

# 3a. Écrire un fichier
mcp fs.write '{"mount_id":"ws","path":"/readme.md","content":"# Hello\nThis is a test.\n"}'
# Attendu : {"path":"/readme.md","bytes":23}

# 3b. Lire le fichier
mcp fs.read '{"mount_id":"ws","path":"/readme.md"}'
# Attendu : contenu avec "# Hello"

# 3c. Stat
mcp fs.stat '{"mount_id":"ws","path":"/readme.md"}'
# Attendu : {"path":"/readme.md","kind":"file","size":23,"mtime":...}

# 3d. Modifier (edit)
mcp fs.read '{"mount_id":"ws","path":"/readme.md"}' > /dev/null   # établit le read guard
mcp fs.edit '{"mount_id":"ws","path":"/readme.md","old_text":"Hello","new_text":"World"}'
# Attendu : diff montrant -Hello +World

# 3e. Vérifier la modification
mcp fs.read '{"mount_id":"ws","path":"/readme.md"}'
# Attendu : "# World"

# 3f. Copier
mcp fs.copy '{"mount_id":"ws","src":"/readme.md","dst":"/readme.bak"}'
# Attendu : {"copied":true}

# 3g. Lister le répertoire
mcp fs.list '{"mount_id":"ws","path":"/"}'
# Attendu : readme.md et readme.bak

# 3h. Déplacer
mcp fs.move '{"mount_id":"ws","src":"/readme.bak","dst":"/archive/readme.bak"}'
# Attendu : {"moved":true}

# 3i. Tree
mcp fs.tree '{"mount_id":"ws","path":"/"}'
# Attendu : arbre avec /readme.md et /archive/readme.bak

# 3j. Supprimer (soft delete vers trash)
mcp fs.delete '{"mount_id":"ws","path":"/archive","recursive":true}'
# Attendu : {"deleted":true}

# 3k. Vérifier l'absence
mcp fs.exists '{"mount_id":"ws","path":"/archive"}'
# Attendu : {"exists":false}
```

**Critères de succès :** tous les retours correspondent, le tree en 3i montre bien les
deux fichiers, l'exists en 3k est false.

---

## Scénario 4 — REST data plane (upload / download / zip)

**But :** valider la parité MCP/REST et les endpoints de transfert de fichiers.

```bash
# 4a. Upload d'un fichier via REST
echo "Hello from REST" > /tmp/hello.txt
curl -sf -X POST "$MCP/api/fs/ws/upload" \
  -H "$AUTH" \
  -F "file=@/tmp/hello.txt;filename=hello.txt" \
  | python3 -m json.tool
# Attendu : {"path":"/hello.txt","bytes":16}

# 4b. Lire le même fichier via MCP (parité)
mcp fs.read '{"mount_id":"ws","path":"/hello.txt"}'
# Attendu : "Hello from REST"

# 4c. Download via REST
curl -sf "$MCP/api/fs/ws/download?path=/hello.txt" \
  -H "$AUTH" -o /tmp/downloaded.txt
cat /tmp/downloaded.txt
# Attendu : "Hello from REST"

# 4d. Download ZIP du volume
curl -sf "$MCP/api/fs/ws/download/zip" \
  -H "$AUTH" -o /tmp/ws.zip
unzip -l /tmp/ws.zip
# Attendu : archive zip contenant hello.txt et readme.md

# 4e. Swagger UI accessible sans token
curl -sf "$MCP/api/docs" | head -5
# Attendu : HTML de la Swagger UI

# 4f. OpenAPI spec
curl -sf "$MCP/api/swagger.json" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['info']['title'], d['info']['version'])"
# Attendu : "mcp-fs ..."
```

**Critères de succès :** upload puis lecture MCP retourne le même contenu, zip valide,
Swagger UI accessible.

---

## Scénario 5 — Recherche et grep

**But :** valider glob, grep et find sur un volume avec plusieurs fichiers.

```bash
# Créer une arborescence de test
mcp fs.write '{"mount_id":"ws","path":"/src/main.rs","content":"fn main() {\n    println!(\"hello\");\n}\n"}'
mcp fs.write '{"mount_id":"ws","path":"/src/lib.rs","content":"pub fn add(a: i32, b: i32) -> i32 { a + b }\n"}'
mcp fs.write '{"mount_id":"ws","path":"/docs/README.md","content":"# Docs\nSee src/lib.rs for the API.\n"}'

# 5a. Glob par extension
mcp fs.glob '{"mount_id":"ws","pattern":"*.rs"}'
# Attendu : main.rs et lib.rs (ordre par mtime decroissant)

# 5b. Grep — mode files (chercher "fn")
mcp fs.grep '{"mount_id":"ws","pattern":"fn","mode":"files"}'
# Attendu : liste de fichiers contenant "fn"

# 5c. Grep — mode content (avec contexte)
mcp fs.grep '{"mount_id":"ws","pattern":"println","mode":"content","context_lines":1}'
# Attendu : la ligne println avec une ligne de contexte avant/après

# 5d. Grep — mode count
mcp fs.grep '{"mount_id":"ws","pattern":"fn","mode":"count"}'
# Attendu : {"file":"/src/main.rs","count":1}, {"file":"/src/lib.rs","count":1}

# 5e. Find definition (tree-sitter)
mcp fs.find_definition '{"mount_id":"ws","name":"add","path":"/src"}'
# Attendu : {"name":"add","path":"/src/lib.rs","line":1,"kind":"function"}

# 5f. Glob avec exclude
mcp fs.glob '{"mount_id":"ws","pattern":"*","exclude":"*.md"}'
# Attendu : aucun .md dans les résultats
```

**Critères de succès :** glob retourne les bons fichiers, grep content montre le contexte,
find_definition localise `add`.

---

## Scénario 6 — Extraction de documents

**But :** tester `fs.extract_text` sur différents formats.

```bash
# 6a. Extraction d'un fichier texte (retour direct, pas de companion)
mcp fs.write '{"mount_id":"ws","path":"/note.txt","content":"Line 1\nLine 2\nLine 3\n"}'
mcp fs.extract_text '{"mount_id":"ws","path":"/note.txt"}'
# Attendu : {"text":"Line 1\nLine 2\nLine 3\n","md_path":null}

# 6b. Extraction d'un HTML (companion .md créé)
mcp fs.write '{"mount_id":"ws","path":"/page.html","content":"<html><body><h1>Title</h1><p>Content here.</p></body></html>"}'
mcp fs.extract_text '{"mount_id":"ws","path":"/page.html"}'
# Attendu : {"text":"# Title\n\nContent here.\n","md_path":"/page.md"}

# Vérifier que le companion existe
mcp fs.exists '{"mount_id":"ws","path":"/page.md"}'
# Attendu : {"exists":true}

# 6c. Extraction d'un CSV
mcp fs.write '{"mount_id":"ws","path":"/data.csv","content":"name,age,city\nAlice,30,Paris\nBob,25,Lyon\n"}'
mcp fs.extract_text '{"mount_id":"ws","path":"/data.csv"}'
# Attendu : tableau Markdown avec les colonnes name/age/city

# 6d. Tentative sur un format non supporté
mcp fs.extract_text '{"mount_id":"ws","path":"/note.txt"}' # deja fait, vérifier le cache
# Deuxieme appel : même résultat, pas de re-extraction (companion mis en cache)
```

**Critères de succès :** companion .md créé pour HTML, tableau markdown pour CSV, pas
d'erreur sur .txt.

---

## Scénario 7 — Éditeur HTML interactif

**But :** ouvrir un fichier HTML dans le navigateur via `doc.open_editor`, éditer dans le
navigateur, vérifier que les changements apparaissent dans le volume, et inversement.

> Prérequis : `--doc` flag au démarrage du serveur. Modifier `run.sh` temporairement :
> `exec ./target/release/mcp-fs serve --config config/local.yaml --doc`
> ou ajouter `doc: { enabled: true }` dans `config/local.yaml`.

```bash
# 7a. Ouvrir un nouveau fichier (sera créé automatiquement)
mcp doc.open_editor '{"mount_id":"ws","path":"/article.html","mode":"doc"}'
# Attendu :
# {
#   "editor_id": "...",
#   "url": "http://127.0.0.1:<port>",
#   "path": "/article.html",
#   "mode": "doc",
#   "created": true
# }
# Le navigateur s'ouvre automatiquement sur l'URL.
```

**Étapes manuelles dans le navigateur :**
1. Taper du texte dans l'éditeur (zone blanche sous le titre)
2. Attendre ~1s (debounce) — l'indicateur "✓ saved" doit apparaître

```bash
# 7b. Vérifier que l'édition est bien persistée dans le volume
mcp fs.read '{"mount_id":"ws","path":"/article.html"}'
# Attendu : le fichier contient le texte tapé dans le navigateur
```

**Étapes manuelles — sync inverse :**
```bash
# 7c. Modifier le fichier depuis le terminal (simule fs.write par le LLM)
mcp fs.read '{"mount_id":"ws","path":"/article.html"}' > /dev/null
mcp fs.edit '{"mount_id":"ws","path":"/article.html","old_text":"Start writing here.","new_text":"Updated by the LLM!"}'
```
Dans le navigateur, après ~500ms : le contenu doit se recharger avec "Updated by the LLM!".

```bash
# 7d. Idempotence : second appel sur le même fichier retourne le même editor_id
FIRST_ID=$(mcp doc.open_editor '{"mount_id":"ws","path":"/article.html","mode":"doc"}' | python3 -c "import sys,json; print(json.loads(sys.stdin.read())['editor_id'])")
SECOND_ID=$(mcp doc.open_editor '{"mount_id":"ws","path":"/article.html","mode":"doc"}' | python3 -c "import sys,json; print(json.loads(sys.stdin.read())['editor_id'])")
[ "$FIRST_ID" = "$SECOND_ID" ] && echo "PASS: same editor_id" || echo "FAIL: different ids"

# 7e. Lister les éditeurs ouverts
mcp doc.list_editors '{}'
# Attendu : article.html dans la liste

# 7f. Ouvrir une présentation (mode slides)
mcp doc.open_editor '{"mount_id":"ws","path":"/slides.html","mode":"slides"}'
# Le navigateur ouvre une page avec nav précédent/suivant
# Chaque <section> = une slide

# 7g. Fermer l'éditeur article
EDITOR_ID=$(mcp doc.list_editors '{}' | python3 -c "
import sys,json
eds = json.loads(sys.stdin.read())['editors']
print(next(e['editor_id'] for e in eds if 'article' in e['path']))
")
mcp doc.close_editor "{\"editor_id\":\"$EDITOR_ID\"}"
# Attendu : {"closed":true}

# 7h. Vérifier que le port est libéré (l'URL ne répond plus)
URL=$(mcp doc.list_editors '{}' | ...)  # l'éditeur n'est plus là
```

**Critères de succès :** browser s'ouvre, save debounce visible, reload WS en <1s, idempotence, close libère le port.

---

## Scénario 8 — SQLite dans le volume

**But :** créer une base SQLite dans un volume et l'interroger via les outils `db.*`.

```bash
# 8a. Créer un CSV, le requêter avec DataFusion
mcp fs.write '{"mount_id":"ws","path":"/sales.csv","content":"product,qty,price\nWidget,10,9.99\nGadget,5,24.99\nDoohickey,3,4.99\n"}'

mcp db.query '{"mount_id":"ws","path":"/sales.csv","sql":"SELECT product, qty * price AS total FROM sales ORDER BY total DESC"}'
# Attendu : Gadget 124.95 / Widget 99.9 / Doohickey 14.97

# 8b. Schema d'un CSV
mcp db.schema '{"mount_id":"ws","path":"/sales.csv"}'
# Attendu : colonnes product(Utf8), qty(Int64), price(Float64)

# 8c. Sample (3 lignes)
mcp db.sample '{"mount_id":"ws","path":"/sales.csv","n":2}'
# Attendu : 2 premières lignes
```

**Critères de succès :** requête SQL retourne les bons totaux, schema correct.

---

## Scénario 9 — Opérations Git

> Prérequis : `git.enabled: true` dans `config/local.yaml` et redémarrer le serveur.

```bash
# 9a. Initialiser un dépôt dans le projet ws
mcp git.init '{"mount_id":"ws"}'
# Attendu : {"initialized":true,"mount_id":"ws"}

# 9b. Cloner depuis un client git externe (terminal séparé)
git clone http://127.0.0.1:5002/git/ws/ /tmp/ws-repo
# Doit réussir (dépôt vide)

cd /tmp/ws-repo
git config user.email "you@example.com"
git config user.name "You"

# 9c. Créer un commit et pousser
echo "# Project WS" > README.md
git add README.md
git commit -m "init"
git push origin main
# Attendu : push OK

# 9d. Vérifier que le fichier est indexé dans le volume
mcp git.log '{"mount_id":"ws"}'
# Attendu : un commit avec "init"

# 9e. Fetch depuis un second clone
git clone http://127.0.0.1:5002/git/ws/ /tmp/ws-repo2
cat /tmp/ws-repo2/README.md
# Attendu : "# Project WS"

# 9f. Statut des refs
mcp git.refs '{"mount_id":"ws"}'
# Attendu : refs/heads/main → sha du commit
```

**Critères de succès :** push/fetch round-trip sans erreur, `git.log` reflète le commit,
second clone contient le fichier.

---

## Scénario 10 — Sécurité et isolation

**But :** vérifier que l'isolation entre projets et l'ACL fonctionnent end-to-end.

```bash
# Créer deux projets avec deux utilisateurs distincts
mcp admin.create_project '{"project_id":"proj-a","owner":"you@example.com"}'
mcp admin.create_project '{"project_id":"proj-b","owner":"you@example.com"}'

# Token pour alice (non membre d'aucun des deux projets)
TOKEN_ALICE=$(./target/release/mcp-fs token alice@example.com --key .keys/jwt.key)

# 10a. Alice ne peut pas lire proj-a
curl -sf -X POST "$MCP/mcp" \
  -H "$H" -H "$A" -H "X-Forwarded-Authorization: Bearer $TOKEN_ALICE" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fs.read","arguments":{"mount_id":"proj-a","path":"/readme.md"}}}'
# Attendu : ERR_FORBIDDEN ou ERR_PROJECT_NOT_FOUND (selon la politique de disclosure)

# 10b. Ajouter alice à proj-a seulement
mcp admin.add_member '{"project_id":"proj-a","person":"alice@example.com"}'

# 10c. Alice peut lire proj-a
mcp fs.write '{"mount_id":"proj-a","path":"/secret.txt","content":"classified"}'
curl -sf -X POST "$MCP/mcp" \
  -H "$H" -H "$A" -H "X-Forwarded-Authorization: Bearer $TOKEN_ALICE" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fs.read","arguments":{"mount_id":"proj-a","path":"/secret.txt"}}}' \
  | grep -o '"text":"[^"]*"'
# Attendu : "classified"

# 10d. Alice ne peut toujours pas lire proj-b
curl -sf -X POST "$MCP/mcp" \
  -H "$H" -H "$A" -H "X-Forwarded-Authorization: Bearer $TOKEN_ALICE" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fs.list","arguments":{"mount_id":"proj-b","path":"/"}}}'
# Attendu : ERR_FORBIDDEN

# 10e. Alice ne peut pas écrire dans proj-a (non owner, juste membre)
# Note : les membres peuvent lire ET écrire (même ACL que l'owner)
# Ce test vérifie qu'un token forgé avec un email différent est rejeté
FORGED="eyJhbGciOiJSUzI1NiJ9.eyJlbWFpbCI6ImFkbWluQGV4YW1wbGUuY29tIn0.invalidsig"
curl -sf -X POST "$MCP/mcp" \
  -H "$H" -H "$A" -H "X-Forwarded-Authorization: Bearer $FORGED" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"admin.list_projects","arguments":{}}}'
# Attendu : 401

# 10f. Quota : écrire trop de données
# (le quota par défaut est 50 MB — pour tester, baisser dans config/local.yaml à 100 bytes et redémarrer)
# safety:
#   write_quota_bytes: 100
mcp fs.write '{"mount_id":"proj-a","path":"/big.txt","content":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}'
# Attendu (avec quota 100) : ERR_QUOTA_EXCEEDED
```

**Critères de succès :** 10a/10d rejettent, 10c accepte, 10e rejette le token forgé.

---

## Scénario 11 — Pandoc (doc.to_docx / doc.to_pptx)

> Prérequis : `pandoc` installé (`brew install pandoc`) et flag `--doc` au démarrage.

```bash
# 11a. Markdown → DOCX
mcp fs.write '{"mount_id":"ws","path":"/report.md","content":"# Rapport\n\n## Résultats\n\nLes résultats sont **excellents**.\n\n| Métrique | Valeur |\n|---|---|\n| Précision | 98% |\n| Rappel | 95% |\n"}'

mcp doc.to_docx '{"mount_id":"ws","src":"/report.md","dst":"/report.docx"}'
# Attendu : {"src":"/report.md","dst":"/report.docx","bytes":<N>}

# Vérifier que le fichier est bien un ZIP (DOCX = ZIP)
mcp fs.read_bytes '{"mount_id":"ws","path":"/report.docx"}' \
  | python3 -c "import sys,json,base64; d=json.loads(sys.stdin.read()); print(base64.b64decode(d['bytes'])[:4])"
# Attendu : b'PK\x03\x04' (signature ZIP)

# 11b. HTML → DOCX
mcp fs.write '{"mount_id":"ws","path":"/page.html","content":"<h1>Titre</h1><p>Un paragraphe <strong>important</strong>.</p>"}'
mcp doc.to_docx '{"mount_id":"ws","src":"/page.html","dst":"/page.docx"}'
# Attendu : {"dst":"/page.docx","bytes":<N>}

# 11c. Markdown → PPTX
mcp fs.write '{"mount_id":"ws","path":"/pres.md","content":"# Slide 1\n\nContenu de la première slide.\n\n---\n\n# Slide 2\n\nContenu de la deuxième slide.\n"}'
mcp doc.to_pptx '{"mount_id":"ws","src":"/pres.md","dst":"/pres.pptx"}'
# Attendu : {"dst":"/pres.pptx","bytes":<N>}

# 11d. Pas d'écrasement sans overwrite
mcp doc.to_docx '{"mount_id":"ws","src":"/report.md","dst":"/report.docx"}'
# Attendu : ERR_ALREADY_EXISTS

# 11e. Écrasement explicite
mcp doc.to_docx '{"mount_id":"ws","src":"/report.md","dst":"/report.docx","overwrite":true}'
# Attendu : succès

# 11f. Extension incorrecte
mcp doc.to_docx '{"mount_id":"ws","src":"/report.md","dst":"/report.pdf"}'
# Attendu : ERR_INVALID_ARGUMENT (extension must be .docx)
```

**Critères de succès :** DOCX démarre par `PK` (signature ZIP), PPTX généré, erreurs
attendues sur no-clobber et mauvaise extension.

---

## Scénario 12 — Agent CLI interactif

**But :** vérifier que l'agent CLI démarre, se connecte au serveur, et exécute une tâche
en langage naturel.

```bash
# Prérequis : token agent
mkdir -p .agent_keys
./target/release/mcp-fs token you@example.com --key .keys/jwt.key > .agent_keys/you

# Démarrer l'agent (démarre le serveur si nécessaire)
./agent.sh --user you
```

**Dans le prompt de l'agent, taper :**

```
> Crée un projet "demo2" dont je suis le propriétaire, écris un fichier /index.md avec
  le contenu "# Demo Project", puis liste le répertoire racine.
```

**Réponse attendue :** l'agent appelle `admin.create_project`, `fs.write`, `fs.list` et
affiche le résultat avec `/index.md`.

```
> Cherche tous les fichiers .md dans le projet demo2 et affiche leur contenu.
```

**Réponse attendue :** appel `fs.glob` puis `fs.read`, affichage de `# Demo Project`.

```
> Supprime le projet demo2.
```

**Réponse attendue :** `admin.delete_project`, confirmation.

**Critères de succès :** l'agent enchaîne les outils sans erreur, les résultats sont
cohérents avec les opérations demandées.

---

## Résumé des critères globaux

| Scénario | Surface | Durée estimée |
|---|---|---|
| 1. Démarrage et santé | JWT, HTTP | 3 min |
| 2. Cycle de vie projet | admin.* | 5 min |
| 3. CRUD fichiers MCP | fs.* tools | 8 min |
| 4. REST data plane | /api/fs | 5 min |
| 5. Recherche et grep | fs.glob, fs.grep, fs.find | 5 min |
| 6. Extraction documents | fs.extract_text | 5 min |
| 7. Éditeur HTML | doc.open/close/list_editors | 10 min |
| 8. SQLite / CSV | db.* | 5 min |
| 9. Git HTTP | git.* | 10 min |
| 10. Sécurité et isolation | ACL, quota, JWT | 8 min |
| 11. Pandoc | doc.to_docx/pptx | 5 min |
| 12. Agent CLI | agent end-to-end | 10 min |

Total : ~1h15 pour la suite complète. Les scénarios 1 à 6 forment le "smoke test"
minimum (~30 min) à dérouler à chaque release.
