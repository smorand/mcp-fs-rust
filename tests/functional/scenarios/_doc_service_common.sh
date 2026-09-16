#!/usr/bin/env bash
# Plomberie partagée des scénarios 25 et 26 (document as a service).
#
# Ces deux scénarios ne passent pas par l'agent LLM : ils vérifient une chaîne
# HTTP exacte (upload multipart avec le flag, puis lecture du compagnon .md), donc
# ils parlent directement au serveur en curl. Le serveur du runner tourne avec
# config/local.yaml où doc_service est désactivé : chaque scénario démarre donc
# sa propre instance, sur son port, avec sa config et son état jetable.
#
# Le nom commence par un souligné : run_all.sh ne source que [0-9][0-9]_*.sh, ce
# fichier est donc chargé explicitement par les scénarios qui en ont besoin.

DS_PID=""
DS_DIR=""
DS_PORT=""
DS_TOKEN=""
DS_PROJ=""

# Le binaire du serveur, construit si le runner ne l'a pas déjà fait.
ds_binary() {
  if [[ ! -x ./target/release/mcp-fs ]]; then
    cargo build --release -p mcp-fs -q || return 1
  fi
  [[ -x ./target/release/mcp-fs ]]
}

# Démarre un serveur dédié. $1 = port, $2 = bloc YAML doc_service (multi-lignes).
# Renvoie 1 si le serveur ne répond pas, pour que l'appelant puisse skip.
ds_start() {
  DS_PORT="$1"
  local doc_block="$2"

  ds_binary || return 1
  if [[ ! -f .keys/jwt.pub ]]; then
    ./target/release/mcp-fs keys --dir .keys >/dev/null 2>&1 || return 1
  fi

  DS_DIR="$(mktemp -d "${TMPDIR:-/tmp}/mcp-fs-docsvc.XXXXXX")"
  cat > "$DS_DIR/config.yaml" <<EOF
server:
  host: 127.0.0.1
  port: $DS_PORT
  mcp_path: /mcp
auth:
  jwt:
    public_key_path: .keys/jwt.pub
    header: X-Forwarded-Authorization
    algorithms: [RS256]
    issuer: web-a2a
    username_claim: email
  admins:
    - admin@example.com
infra:
  meta:
    backend: sqlite
    dir: $DS_DIR/volumes
  blob:
    backend: local
    dir: $DS_DIR/blobs
  admin:
    backend: sqlite
    path: $DS_DIR/admin.db
api:
  enabled: true
$doc_block
EOF

  ./target/release/mcp-fs serve --config "$DS_DIR/config.yaml" \
    >"$DS_DIR/server.log" 2>&1 &
  DS_PID=$!
  for _ in $(seq 1 60); do
    curl -fsS -m 2 "http://127.0.0.1:$DS_PORT/health" >/dev/null 2>&1 && break
    sleep 0.2
  done
  if ! curl -fsS -m 2 "http://127.0.0.1:$DS_PORT/health" >/dev/null 2>&1; then
    return 1
  fi

  DS_TOKEN="$(./target/release/mcp-fs token admin@example.com --key .keys/jwt.key 2>/dev/null)"
  [[ -n "$DS_TOKEN" ]] || return 1

  DS_PROJ="ds-$$"
  ds_mcp admin.create_project "{\"project_id\":\"$DS_PROJ\",\"owner\":\"admin@example.com\"}" \
    >/dev/null
  return 0
}

# Arrête le serveur dédié et efface son état.
ds_stop() {
  if [[ -n "$DS_PID" ]]; then
    kill -TERM "$DS_PID" 2>/dev/null || true
    wait "$DS_PID" 2>/dev/null || true
    DS_PID=""
  fi
  if [[ -n "$DS_DIR" && -d "$DS_DIR" ]]; then
    rm -rf "$DS_DIR"
    DS_DIR=""
  fi
}

# Appelle un outil MCP. $1 = nom, $2 = objet JSON des arguments. Le corps brut est
# renvoyé tel quel (le transport encadre en SSE), les assertions grep dedans.
ds_mcp() {
  curl -sS -m 900 -X POST "http://127.0.0.1:$DS_PORT/mcp" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H "X-Forwarded-Authorization: Bearer $DS_TOKEN" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
    2>/dev/null || true
}

# Upload multipart d'un fichier. $1 = chemin local, $2 = valeur du flag.
# Le timeout est large : une conversion réelle prend des dizaines de secondes.
ds_upload() {
  curl -sS -m 900 -X POST "http://127.0.0.1:$DS_PORT/api/fs/$DS_PROJ/upload" \
    -H "X-Forwarded-Authorization: Bearer $DS_TOKEN" \
    -F "files=@$1" \
    -F "trigger_documentation_service=$2" \
    2>/dev/null || true
}

# Écrit un PDF d'une page contenant "hello mcpfs". Construit ici plutôt que
# committé : le dépôt ne porte aucune fixture binaire.
ds_sample_pdf() {
  printf '%%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj\n4 0 obj<</Length 44>>stream\nBT /F1 24 Tf 20 100 Td (hello mcpfs) Tj ET\nendstream endobj\n5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\ntrailer<</Root 1 0 R>>\n%%%%EOF\n' > "$1"
}
