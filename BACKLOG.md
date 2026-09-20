# Backlog

## Agent CLI

## Outils de génération de documents

### Générateur DOCX depuis Markdown ou HTML

*Implémenté: `doc.to_docx`.*

### Générateur PPTX depuis Markdown ou HTML

*Implémenté: `doc.to_pptx`.*

---

## ~~Éditeur HTML single-page~~ ✅ Implémenté

Trois tools MCP dans la famille `doc.*`:
- `doc.open_editor` — démarre un mini-serveur HTTP+WebSocket par éditeur, crée le fichier si absent (mode `doc` ou `slides`), ouvre le navigateur, retourne `editor_id` + URL.
- `doc.close_editor` — arrête l'éditeur et libère le port.
- `doc.list_editors` — liste les éditeurs actifs.

Sync bidirectionnel: browser → volume via WS `save`, volume → browser via poll mtime 500ms + WS `reload`.
Activé avec le flag `--doc` (même feature que `doc.to_docx` / `doc.to_pptx`).
17 tests dans `tools::editor`.

---

## Support GitHub complet et alimentation du magasin de jetons git

| | |
|---|---|
| **Ouvert le** | 2026-09-20 |
| **Origine** | Intégration Graph Studio, lot 3 « Leonhard générateur de specs ». Graph Studio doit lire un dépôt de code pour étayer chaque affirmation d'une spécification par une citation `fichier:ligne` |
| **Type** | deux défauts, dont un bug latent indépendant de l'appelant |
| **Statut** | à spécifier |

### Défaut 1 : la déduction de fournisseur est faite par sous-chaîne d'URL

`remote_clone` déduit le fournisseur de l'URL seule (`crates/mcp-fs/src/tools/git.rs:684-691`) :

```rust
let provider = if lower.contains("github.com") { Some("github") }
               else if lower.contains("gitlab") { Some("gitlab") }
               else { None };
```

Deux conséquences, symétriques :

* **Trop étroit.** `github.ibm.com` ne contient pas `github.com`. Tout GitHub
  Enterprise Server tombe sur `provider = None`, donc **aucun jeton n'est jamais
  cherché** et `auth` vaut `"anonymous"` (`git.rs:693-706`). Un dépôt privé sur
  GHES est inclonable, et le flux d'appareil n'y change rien : le jeton stocké ne
  serait pas consulté. Azure DevOps est dans le même cas.
* **Trop large.** `lower.contains("gitlab")` classe `gitlab` n'importe quel dépôt
  dont le nom ou le chemin contient cette chaîne, sur n'importe quel hôte. Par
  exemple `https://exemple.test/miroirs/mygitlab-mirror.git`.

Piste : une correspondance **hôte vers fournisseur** configurable, validée au
démarrage, avec les défauts actuels préservés (`github.com` vers github,
`gitlab.com` vers gitlab). Le nom d'hôte est extrait de l'URL, pas cherché en
sous-chaîne.

### Défaut 2 : le magasin de jetons n'a qu'un seul remplisseur

Le jeton est consommé comme un **simple identifiant HTTPS**, rien d'OAuth
(`git.rs:1011-1014`) :

```rust
// The provider expects the token as the password; "oauth2" is the
// conventional username for both GitHub and GitLab.
.credentials(move |_url, _user, _types| git2::Cred::userpass_plaintext("oauth2", &t));
```

Donc **un PAT fonctionne à l'identique**. Le flux d'appareil de `git.auth` n'est
qu'une façon de remplir le magasin, pas une exigence du clone. Or c'est
aujourd'hui la seule : `OAuthTokenStore::store_token` est déjà public
(`crates/mcp-fs/src/git/oauth/store.rs:115`) mais n'est exposé par aucun outil.

Un appelant qui détient déjà le PAT de la personne ne peut donc pas le confier au
serveur, et doit imposer à l'utilisateur un flux d'appareil interactif pour un
jeton qu'il possédait déjà.

Piste : un outil d'alimentation `(personne, fournisseur, jeton)` appelant
`store_token`. Le jeton reste chiffré au repos en AES-256-GCM sous
`MCPFS_TOKEN_KEY`, comme ceux du flux d'appareil, et **toutes** les opérations git
en profitent, pas seulement le clone.

**Préférer un outil additif à un paramètre `token` sur `git.remote_clone`** : la
signature de cet outil est un contrat épinglé (`tool-contract-golden.json`, test
`git_remote_clone_schema_matches_the_contract`, `git.rs:1194`), et un jeton en
paramètre sur un outil appelé souvent est une surface de fuite de plus dans les
traces et le journal d'audit.

### Écrans web : oui pour les jetons, non pour la configuration d'instance

Un écran de gestion de **son** jeton par personne est légitime, et
`doc.open_editor` fournit déjà le patron (mini-serveur HTTP par session, ouverture
du navigateur).

En revanche la configuration d'instance (correspondance hôte vers fournisseur, URL
de base GHES) devrait rester en fichier **validé au démarrage**. Le serveur tient
la propriété « misconfiguration fails at boot, not on first use » (`README.md:140`),
et un routage de confiance éditable à chaud la casse. Un écran peut la lire, pas
l'écrire.

### Impact sur le contrat d'outils

Additif : un outil de plus, `TOOL_CONTRACT.txt` et `tool-contract-golden.json` à
regénérer. La signature de `git.remote_clone` ne bouge pas.

### Consommateur et séquencement

Graph Studio détient déjà un PAT GitHub chiffré par utilisateur
(`src/graph_studio/auth/provider.py:42-50`). Sans ces deux changements, son lecteur
de code reste en mode dégradé, limité aux dépôts publics et aux hôtes `github.com`
ou nommés `gitlab`, ce qui plafonne le verdict de sa barrière d'implémentabilité.
Les deux changements doivent atterrir **avant** la tranche de Graph Studio qui en
dépend.
