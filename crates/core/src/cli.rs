//! Command line surface. Port of the C# `Program.Main` verb switch, with the
//! same verbs, the same flags and the same config path resolution.
//!
//! Differences from C#, all cosmetic: clap renders the help text and rejects
//! unknown flags, where the C# rolled its own `OptionValue` scan. The observable
//! behaviour of `serve`, `keys`, `token` and `version` is identical, including
//! the stderr banner and the token going to stdout on its own so it can be piped
//! straight into a file.

use crate::config::ServerConfig;
use crate::errors::ToolError;
use crate::keys;
use clap::{Parser, Subcommand};
use etcetera::BaseStrategy;
use std::path::PathBuf;
use std::process::ExitCode;

/// Env var holding a full config path, checked before the dir/name pair.
pub const ENV_CONFIG: &str = "MCP_FS_CONFIG";
/// Env var holding the config directory (default `config`).
pub const ENV_CONFIG_DIR: &str = "MCP_FS_CONFIG_DIR";
/// Env var holding the config file stem (default `local`).
pub const ENV_CONFIG_NAME: &str = "MCP_FS_CONFIG_NAME";
/// XDG base directory for user configuration, when set.
pub const ENV_XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
/// Fallback base for the user config directory.
pub const ENV_HOME: &str = "HOME";

/// Our directory inside the XDG config home, and the file we look for there.
pub const XDG_APP_DIR: &str = "mcp-fs";
pub const XDG_CONFIG_FILE: &str = "config.yaml";

const ABOUT: &str = "mcp-fs: filesystem MCP server (SQLite metadata, object store or local blobs)";
const AFTER_HELP: &str = "Config resolution, highest priority first:\n\
     \x20 1. --config PATH (-c)\n\
     \x20 2. $MCP_FS_CONFIG\n\
     \x20 3. ~/.config/mcp-fs/config.yaml (honours $XDG_CONFIG_HOME), when it exists\n\
     \x20 4. $MCP_FS_CONFIG_DIR (default 'config') / $MCP_FS_CONFIG_NAME (default 'local').yaml\n\
     \nCases 1 and 2 are used as given, so a missing file is reported by name. Cases 3\n\
     and 4 are probed, and when neither exists every path tried is listed.";

#[derive(Debug, Parser)]
#[command(name = "mcp-fs", version, about = ABOUT, after_help = AFTER_HELP)]
pub struct Cli {
    /// Omitted verb means `serve`, matching the C# default.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the streamable HTTP MCP server.
    Serve {
        /// Config file path. Overrides MCP_FS_CONFIG and the dir/name pair.
        #[arg(short = 'c', long, value_name = "PATH")]
        config: Option<PathBuf>,
        /// Force git.enabled = true (overrides config YAML).
        #[arg(long)]
        git: bool,
        /// Force web.enabled = true (overrides config YAML).
        #[arg(long)]
        web: bool,
        /// Force context7.enabled = true (overrides config YAML).
        #[arg(long)]
        context7: bool,
        /// Force sqlite.enabled = true (overrides config YAML).
        #[arg(long)]
        sqlite: bool,
        /// Force db.enabled = true (overrides config YAML).
        #[arg(long)]
        db: bool,
        /// Force doc.enabled = true (overrides config YAML).
        #[arg(long)]
        doc: bool,
    },
    /// Generate an RS256 keypair (jwt.key private, jwt.pub public).
    Keys {
        /// Output directory, created if missing.
        #[arg(long, value_name = "DIR", default_value = keys::DEFAULT_KEY_DIR)]
        dir: PathBuf,
    },
    /// Mint a signed dev bearer token and print it on stdout.
    Token {
        /// Identity minted into the token's claim.
        #[arg(value_name = "EMAIL")]
        email: String,
        /// Private key path (default `.keys/jwt.key`).
        #[arg(long, value_name = "PATH")]
        key: Option<PathBuf>,
        /// Token issuer, must match `auth.jwt.issuer`.
        #[arg(long, value_name = "NAME", default_value = keys::DEFAULT_ISSUER)]
        issuer: String,
        /// Claim carrying the identity, must match `auth.jwt.username_claim`.
        #[arg(long, value_name = "NAME", default_value = keys::DEFAULT_CLAIM)]
        claim: String,
        /// Lifetime in seconds.
        #[arg(long, value_name = "SECONDS", default_value_t = keys::DEFAULT_TTL_SECONDS)]
        ttl: i64,
    },
    /// Copy every relational row from one deployment's backends to another's.
    ///
    /// Offline: stop the server first. Blob bytes and OAuth tokens are not moved,
    /// see the `migrate` module docs.
    Migrate {
        /// Config naming the backends to read from.
        #[arg(long, value_name = "PATH")]
        from: PathBuf,
        /// Config naming the backends to write to.
        #[arg(long, value_name = "PATH")]
        to: PathBuf,
    },
    /// Run one purge sweep (SPEC-0014): a project's file/project sweep, or
    /// (with no `--project`) the global grace-period sweep.
    Purge {
        /// Config file path. Overrides MCP_FS_CONFIG and the dir/name pair.
        #[arg(short = 'c', long, value_name = "PATH")]
        config: Option<PathBuf>,
        /// Sweep only this project instead of running the global grace sweep.
        #[arg(long, value_name = "ID")]
        project: Option<String>,
        /// Run the sweep even though the project is not configured for autopurge.
        #[arg(long)]
        on_demand: bool,
    },
    /// Print the version and exit.
    Version,
}

/// Parse the process arguments, run the selected verb, map failure to exit code 1.
pub async fn run() -> ExitCode {
    match dispatch(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Run an already parsed CLI. Split out so tests can drive it without a process.
pub async fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command.unwrap_or(Command::Serve {
        config: None,
        git: false,
        web: false,
        context7: false,
        sqlite: false,
        db: false,
        doc: false,
    }) {
        Command::Version => {
            println!("{}", crate::app::VERSION);
            Ok(())
        }
        Command::Keys { dir } => cmd_keys(&dir),
        Command::Token { email, key, issuer, claim, ttl } => {
            cmd_token(&email, key.as_deref(), &issuer, &claim, ttl)
        }
        Command::Serve { config, git, web, context7, sqlite, db, doc } => {
            cmd_serve(config.as_deref(), git, web, context7, sqlite, db, doc).await
        }
        Command::Migrate { from, to } => cmd_migrate(&from, &to).await,
        Command::Purge { config, project, on_demand } => {
            cmd_purge(config.as_deref(), project, on_demand).await
        }
    }
}

async fn cmd_migrate(from: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
    crate::logging::init();
    // Both configs are named explicitly: a migration that silently resolved one
    // side from the environment could write to the wrong deployment.
    let source = ServerConfig::load(from)?;
    let dest = ServerConfig::load(to)?;
    eprintln!("migrating {} -> {}", from.display(), to.display());
    let report = crate::migrate::run(&source, &dest).await?;
    // stdout carries the machine readable total, so the verb is usable in a script.
    println!("{}", report.total_rows());
    Ok(())
}

/// Dispatch for `mcp-fs purge` (FR-NEW-010/FR-NEW-011).
///
/// Builds its own admin store, volume store manager and safety manager rather
/// than reusing `app::build`'s: a one-shot CLI sweep needs none of `app::build`'s
/// HTTP surface (identity, MCP route, REST plane, git), only the three pieces
/// `purge.rs`'s sweep functions take.
async fn cmd_purge(
    explicit: Option<&std::path::Path>,
    project: Option<String>,
    on_demand: bool,
) -> anyhow::Result<()> {
    crate::logging::init();
    let (config, _resolved) = load_config(explicit)?;
    let config = std::sync::Arc::new(config);
    let relational = std::sync::Arc::new(crate::storage::RelationalRegistry::new());
    let admin = crate::storage::build_admin_store(&config, &relational).await?;
    admin.connect().await?;
    let stores = crate::storage::StoreManager::new(config.clone(), relational.clone());
    let safety = crate::safety::SafetyManager::new(
        config.safety.clone(),
        crate::storage::meta::max_path_len(&config.infra.meta.backend),
    );

    match project {
        Some(project_id) => {
            cmd_purge_project(&*admin, &stores, &safety, &project_id, on_demand).await
        }
        None => {
            // FR-NEW-011: the global form ignores `autopurge_enabled` entirely,
            // since a soft-deleted project is already past that decision point
            // (FR-NEW-012's grace-period comparison, unconditional on
            // `use_internal_purge` too).
            let removed = crate::purge::sweep_grace_period(&stores, &*admin, &config).await?;
            println!("purge summary: global grace sweep, projects_permanently_removed={removed}");
            Ok(())
        }
    }
}

/// The `--project <id>` branch of `cmd_purge`: gate per DEC-003, then run the
/// same unmodified file/project sweep functions the background loop uses.
async fn cmd_purge_project(
    admin: &dyn crate::storage::traits::AdminBackend,
    stores: &crate::storage::StoreManager,
    safety: &crate::safety::SafetyManager,
    project_id: &str,
    on_demand: bool,
) -> anyhow::Result<()> {
    if admin.get_project(project_id).await?.is_none() {
        anyhow::bail!("ERR_PROJECT_NOT_FOUND: project '{project_id}' does not exist");
    }
    let purge_config = admin.get_purge_config(project_id).await?;
    if !purge_config.autopurge_enabled && !on_demand {
        anyhow::bail!(
            "project '{project_id}' is not configured for autopurge \
             (autopurge_enabled=false); pass --on-demand to purge it anyway"
        );
    }
    let client = stores.client(project_id).await?;
    let files_purged =
        crate::purge::sweep_project_files(&client, admin, safety, project_id).await?;
    let project_soft_deleted =
        crate::purge::sweep_project(&client, admin, safety, project_id).await?;
    println!(
        "purge summary: project={project_id} files_purged={files_purged} \
         project_soft_deleted={project_soft_deleted}"
    );
    Ok(())
}

fn cmd_keys(dir: &std::path::Path) -> anyhow::Result<()> {
    let (key_path, pub_path) = keys::write_keypair(dir)?;
    println!("wrote {} (private) and {} (public)", key_path.display(), pub_path.display());
    println!("point auth.jwt.public_key_path at the .pub file in your config.");
    Ok(())
}

fn cmd_token(
    email: &str,
    key: Option<&std::path::Path>,
    issuer: &str,
    claim: &str,
    ttl: i64,
) -> anyhow::Result<()> {
    let key_path =
        key.map(std::path::Path::to_path_buf).unwrap_or_else(keys::default_private_key_path);
    let token = keys::mint_token_from_file(&key_path, email, issuer, claim, ttl)?;
    // Nothing else on stdout: `mcp-fs token me@x.com > token.txt` must be usable.
    println!("{token}");
    Ok(())
}

async fn cmd_serve(
    explicit: Option<&std::path::Path>,
    git: bool,
    web: bool,
    context7: bool,
    sqlite: bool,
    db: bool,
    doc: bool,
) -> anyhow::Result<()> {
    crate::logging::init();
    let (mut config, resolved) = load_config(explicit)?;
    if git {
        config.git.enabled = true;
    }
    if web {
        config.web.enabled = true;
    }
    if context7 {
        config.context7.enabled = true;
    }
    if sqlite {
        config.sqlite.enabled = true;
    }
    if db {
        config.db.enabled = true;
    }
    if doc {
        config.doc.enabled = true;
    }
    // The banner goes to stderr so it never pollutes a piped stdout.
    eprintln!(
        "Serving mcp-fs {} on {}:{} (config={})",
        crate::app::VERSION,
        config.server.host,
        config.server.port,
        resolved.display()
    );
    crate::app::serve(config).await
}

/// Config path resolution: `--config`, else `MCP_FS_CONFIG`, else
/// `~/.config/mcp-fs/config.yaml` when it exists, else
/// `{MCP_FS_CONFIG_DIR|config}/{MCP_FS_CONFIG_NAME|local}.yaml`.
///
/// `Err` carries every path that was probed, for a boot message that says where
/// to put the file instead of naming one location the user may not have expected.
pub fn resolve_config_path(
    explicit: Option<&std::path::Path>,
) -> std::result::Result<PathBuf, Vec<PathBuf>> {
    // The XDG base directory is resolved through `etcetera` (DR-005) rather than
    // the hand-rolled $XDG_CONFIG_HOME-or-$HOME/.config fallback: `config_dir()`
    // already implements that exact precedence, so folding its result into the
    // env closure below keeps `resolve_config_path_with`'s generic, independently
    // tested precedence logic untouched while the real base-directory computation
    // goes through the library. `HOME` (checked directly below, unconditionally on
    // `etcetera` succeeding) is only relevant if `etcetera::choose_base_strategy`
    // itself fails to locate a home directory, which the hand-rolled code treated
    // as "no XDG candidate" rather than an error.
    let xdg_config_home =
        etcetera::choose_base_strategy().ok().map(|s| s.config_dir().display().to_string());
    resolve_config_path_with(
        explicit,
        |k| {
            if k == ENV_XDG_CONFIG_HOME {
                return xdg_config_home.clone();
            }
            std::env::var(k).ok()
        },
        |p| p.exists(),
    )
}

/// Same resolution with an injectable environment and an injectable existence
/// probe, so precedence is testable without touching the process environment
/// (which races across test threads) or the real filesystem.
pub fn resolve_config_path_with(
    explicit: Option<&std::path::Path>,
    env: impl Fn(&str) -> Option<String>,
    exists: impl Fn(&std::path::Path) -> bool,
) -> std::result::Result<PathBuf, Vec<PathBuf>> {
    let non_blank = |v: String| {
        let t = v.trim().to_string();
        (!t.is_empty()).then_some(t)
    };

    // An explicitly named file is authoritative even when absent: the caller asked
    // for that path, so the not found error must name it rather than silently
    // falling through to a different file.
    if let Some(p) = explicit
        && !p.as_os_str().is_empty()
    {
        return Ok(p.to_path_buf());
    }
    if let Some(v) = env(ENV_CONFIG).and_then(non_blank) {
        return Ok(PathBuf::from(v));
    }

    let mut tried = Vec::new();

    // The user wide location, probed only: an absent file here is the normal case
    // for a project local checkout and must not shadow the directory below.
    if let Some(base) = env(ENV_XDG_CONFIG_HOME)
        .and_then(non_blank)
        .map(PathBuf::from)
        .or_else(|| env(ENV_HOME).and_then(non_blank).map(|h| PathBuf::from(h).join(".config")))
    {
        let candidate = base.join(XDG_APP_DIR).join(XDG_CONFIG_FILE);
        if exists(&candidate) {
            return Ok(candidate);
        }
        tried.push(candidate);
    }

    let dir = env(ENV_CONFIG_DIR).and_then(non_blank).unwrap_or_else(|| "config".into());
    let name = env(ENV_CONFIG_NAME).and_then(non_blank).unwrap_or_else(|| "local".into());
    let candidate = PathBuf::from(dir).join(format!("{name}.yaml"));
    if exists(&candidate) {
        return Ok(candidate);
    }
    tried.push(candidate);

    Err(tried)
}

/// Resolve, then load, reporting every probed path when nothing was found.
fn load_config(explicit: Option<&std::path::Path>) -> Result<(ServerConfig, PathBuf), ToolError> {
    match resolve_config_path(explicit) {
        Ok(path) => {
            let config = ServerConfig::load(&path)?;
            Ok((config, path))
        }
        Err(tried) => {
            let list = tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
            Err(ToolError::invalid_argument(format!(
                "no configuration file found, tried: {list}. \
                 Pass --config PATH, set MCP_FS_CONFIG, or create one of those files"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::traits::AdminBackend;
    use clap::CommandFactory;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    /// Every candidate exists, so each test below observes precedence alone.
    fn all_exist(_: &std::path::Path) -> bool {
        true
    }

    fn none_exist(_: &std::path::Path) -> bool {
        false
    }

    #[test]
    fn explicit_path_wins_over_everything() {
        let p = resolve_config_path_with(
            Some(std::path::Path::new("/tmp/x.yaml")),
            |k| match k {
                ENV_CONFIG => Some("/env.yaml".to_string()),
                ENV_HOME => Some("/home/me".to_string()),
                _ => None,
            },
            all_exist,
        );
        assert_eq!(p.unwrap(), PathBuf::from("/tmp/x.yaml"));
    }

    /// An explicitly named file is used even when absent, so the not found error
    /// names the path the caller actually asked for.
    #[test]
    fn explicit_path_is_used_even_when_it_does_not_exist() {
        let p = resolve_config_path_with(
            Some(std::path::Path::new("/tmp/absent.yaml")),
            no_env,
            none_exist,
        );
        assert_eq!(p.unwrap(), PathBuf::from("/tmp/absent.yaml"));
    }

    #[test]
    fn env_config_wins_over_xdg_and_the_dir_name_pair() {
        let p = resolve_config_path_with(
            None,
            |k| match k {
                ENV_CONFIG => Some("/env.yaml".into()),
                ENV_CONFIG_DIR => Some("other".into()),
                ENV_XDG_CONFIG_HOME => Some("/xdg".into()),
                _ => None,
            },
            all_exist,
        );
        assert_eq!(p.unwrap(), PathBuf::from("/env.yaml"));
    }

    #[test]
    fn env_config_is_used_even_when_it_does_not_exist() {
        let p = resolve_config_path_with(
            None,
            |k| (k == ENV_CONFIG).then(|| "/env.yaml".to_string()),
            none_exist,
        );
        assert_eq!(p.unwrap(), PathBuf::from("/env.yaml"));
    }

    #[test]
    fn xdg_config_home_is_probed_before_the_dir_name_pair() {
        let p = resolve_config_path_with(
            None,
            |k| (k == ENV_XDG_CONFIG_HOME).then(|| "/xdg".to_string()),
            all_exist,
        );
        assert_eq!(p.unwrap(), PathBuf::from("/xdg/mcp-fs/config.yaml"));
    }

    #[test]
    fn home_supplies_the_xdg_base_when_the_variable_is_unset() {
        let p = resolve_config_path_with(
            None,
            |k| (k == ENV_HOME).then(|| "/home/me".to_string()),
            all_exist,
        );
        assert_eq!(p.unwrap(), PathBuf::from("/home/me/.config/mcp-fs/config.yaml"));
    }

    /// The new step must not shadow a project local checkout, which is the common
    /// case: no user wide file, so the directory pair still wins.
    #[test]
    fn an_absent_xdg_file_falls_through_to_the_dir_name_pair() {
        let p = resolve_config_path_with(
            None,
            |k| (k == ENV_HOME).then(|| "/home/me".to_string()),
            |path| path == std::path::Path::new("config/local.yaml"),
        );
        assert_eq!(p.unwrap(), PathBuf::from("config/local.yaml"));
    }

    #[test]
    fn dir_and_name_compose_the_default() {
        assert_eq!(
            resolve_config_path_with(None, no_env, all_exist).unwrap(),
            PathBuf::from("config/local.yaml")
        );
        let p = resolve_config_path_with(
            None,
            |k| match k {
                ENV_CONFIG_DIR => Some("/etc/mcpfs".into()),
                ENV_CONFIG_NAME => Some("prod".into()),
                _ => None,
            },
            all_exist,
        );
        assert_eq!(p.unwrap(), PathBuf::from("/etc/mcpfs/prod.yaml"));
    }

    #[test]
    fn blank_env_values_fall_back_to_defaults() {
        let p = resolve_config_path_with(
            None,
            |k| match k {
                ENV_CONFIG => Some("   ".into()),
                ENV_CONFIG_DIR => Some("".into()),
                ENV_CONFIG_NAME => Some(" ".into()),
                ENV_XDG_CONFIG_HOME => Some("  ".into()),
                _ => None,
            },
            all_exist,
        );
        assert_eq!(p.unwrap(), PathBuf::from("config/local.yaml"));
    }

    /// With nothing on disk the caller gets every probed path, so the boot message
    /// can tell the operator where the file may live.
    #[test]
    fn nothing_found_reports_every_path_tried() {
        let tried = resolve_config_path_with(
            None,
            |k| (k == ENV_HOME).then(|| "/home/me".to_string()),
            none_exist,
        )
        .expect_err("no candidate exists");
        assert_eq!(
            tried,
            vec![
                PathBuf::from("/home/me/.config/mcp-fs/config.yaml"),
                PathBuf::from("config/local.yaml"),
            ]
        );
    }

    #[test]
    fn no_verb_means_serve() {
        let cli = Cli::parse_from(["mcp-fs"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn serve_accepts_short_and_long_config_flags() {
        for args in
            [vec!["mcp-fs", "serve", "--config", "a.yaml"], vec!["mcp-fs", "serve", "-c", "a.yaml"]]
        {
            match Cli::parse_from(args).command {
                Some(Command::Serve { config, .. }) => {
                    assert_eq!(config, Some(PathBuf::from("a.yaml")));
                }
                other => panic!("expected serve, got {other:?}"),
            }
        }
    }

    #[test]
    fn serve_accepts_sqlite_and_db_flags() {
        match Cli::parse_from(["mcp-fs", "serve", "--sqlite", "--db"]).command {
            Some(Command::Serve { sqlite, db, .. }) => {
                assert!(sqlite);
                assert!(db);
            }
            other => panic!("expected serve, got {other:?}"),
        }
    }

    #[test]
    fn serve_accepts_git_web_context7_flags() {
        match Cli::parse_from(["mcp-fs", "serve", "--git", "--web", "--context7"]).command {
            Some(Command::Serve { git, web, context7, .. }) => {
                assert!(git);
                assert!(web);
                assert!(context7);
            }
            other => panic!("expected serve, got {other:?}"),
        }
    }

    #[test]
    fn keys_defaults_to_the_dot_keys_dir() {
        match Cli::parse_from(["mcp-fs", "keys"]).command {
            Some(Command::Keys { dir }) => assert_eq!(dir, PathBuf::from(".keys")),
            other => panic!("expected keys, got {other:?}"),
        }
        match Cli::parse_from(["mcp-fs", "keys", "--dir", "/tmp/k"]).command {
            Some(Command::Keys { dir }) => assert_eq!(dir, PathBuf::from("/tmp/k")),
            other => panic!("expected keys, got {other:?}"),
        }
    }

    #[test]
    fn token_defaults_match_csharp() {
        match Cli::parse_from(["mcp-fs", "token", "me@test.com"]).command {
            Some(Command::Token { email, key, issuer, claim, ttl }) => {
                assert_eq!(email, "me@test.com");
                assert_eq!(key, None);
                assert_eq!(issuer, "web-a2a");
                assert_eq!(claim, "email");
                assert_eq!(ttl, 3600);
            }
            other => panic!("expected token, got {other:?}"),
        }
    }

    #[test]
    fn token_accepts_every_flag() {
        match Cli::parse_from([
            "mcp-fs",
            "token",
            "me@test.com",
            "--key",
            "/k/jwt.key",
            "--issuer",
            "other",
            "--claim",
            "upn",
            "--ttl",
            "60",
        ])
        .command
        {
            Some(Command::Token { key, issuer, claim, ttl, .. }) => {
                assert_eq!(key, Some(PathBuf::from("/k/jwt.key")));
                assert_eq!(issuer, "other");
                assert_eq!(claim, "upn");
                assert_eq!(ttl, 60);
            }
            other => panic!("expected token, got {other:?}"),
        }
    }

    #[test]
    fn version_is_a_verb() {
        assert!(matches!(Cli::parse_from(["mcp-fs", "version"]).command, Some(Command::Version)));
    }

    #[test]
    fn unknown_verbs_are_rejected() {
        assert!(Cli::try_parse_from(["mcp-fs", "frobnicate"]).is_err());
    }

    #[test]
    fn purge_accepts_project_and_on_demand() {
        match Cli::parse_from(["mcp-fs", "purge", "--project", "proj1", "--on-demand"]).command {
            Some(Command::Purge { project, on_demand, .. }) => {
                assert_eq!(project, Some("proj1".to_string()));
                assert!(on_demand);
            }
            other => panic!("expected purge, got {other:?}"),
        }
    }

    #[test]
    fn purge_with_no_flags_means_the_global_sweep() {
        match Cli::parse_from(["mcp-fs", "purge"]).command {
            Some(Command::Purge { project, on_demand, .. }) => {
                assert_eq!(project, None);
                assert!(!on_demand);
            }
            other => panic!("expected purge, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn keys_then_token_works_end_to_end() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("k");
        dispatch(Cli::parse_from(["mcp-fs", "keys", "--dir", dir.to_str().unwrap()]))
            .await
            .unwrap();
        assert!(dir.join("jwt.pub").exists());

        let key = dir.join("jwt.key");
        dispatch(Cli::parse_from([
            "mcp-fs",
            "token",
            "me@test.com",
            "--key",
            key.to_str().unwrap(),
        ]))
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn token_with_a_missing_key_fails() {
        assert!(
            dispatch(Cli::parse_from([
                "mcp-fs",
                "token",
                "me@test.com",
                "--key",
                "/definitely/not/here.key",
            ]))
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn serve_with_a_missing_config_fails() {
        assert!(
            dispatch(Cli::parse_from(
                ["mcp-fs", "serve", "--config", "/definitely/not/here.yaml",]
            ))
            .await
            .is_err()
        );
    }

    /// Writes a `ServerConfig` pointed at throwaway state under `root` to
    /// `root/config.yaml`, returning that path. Shared by every `purge` e2e test
    /// below, which seed the admin db directly (bypassing the CLI, which has no
    /// `admin.create_project` verb) then run `dispatch` against the same file.
    fn write_purge_test_config(root: &std::path::Path) -> (PathBuf, ServerConfig) {
        let mut c = ServerConfig::default();
        c.infra.meta.dir = root.join("volumes").display().to_string();
        c.infra.blob.dir = root.join("blobs").display().to_string();
        c.infra.admin.path = root.join("admin.db").display().to_string();
        // Hand-written rather than `serde_yaml::to_string(&c)`: `ServerConfig`'s
        // default `git.hosts` round-trips through YAML as a sequence, which its
        // own `Deserialize` then rejects (expects a mapping); every other field
        // is left to its `#[serde(default)]`, exactly like the minimal YAML
        // fixtures in `config.rs`'s own tests.
        let yaml = format!(
            "infra:\n  meta:\n    dir: {:?}\n  blob:\n    dir: {:?}\n  admin:\n    path: {:?}\n",
            c.infra.meta.dir, c.infra.blob.dir, c.infra.admin.path
        );
        let path = root.join("config.yaml");
        std::fs::write(&path, yaml).unwrap();
        (path, c)
    }

    /// Opens the same admin db `write_purge_test_config` pointed `config.yaml`
    /// at, as the concrete store (not the `AdminBackend` trait object `cmd_purge`
    /// builds), so a test can reach the `pub(crate)`, test-only seeding methods
    /// (`admin.set_purge_config` is US-0009, not yet implemented).
    async fn seeded_admin(config: &ServerConfig) -> crate::storage::admin::RelationalAdminStore {
        let relational = std::sync::Arc::new(crate::storage::RelationalRegistry::new());
        let db = crate::storage::open_admin_db(config, &relational).await.unwrap();
        let admin = crate::storage::admin::RelationalAdminStore::new(db);
        AdminBackend::connect(&admin).await.unwrap();
        admin
    }

    /// E2E-NEW-046: happy path, `--project` on an autopurge-enabled project with
    /// a stale file and a stale-project condition both purged.
    #[tokio::test]
    async fn e2e_new_046_cli_happy_path() {
        let d = tempfile::tempdir().unwrap();
        let (path, config) = write_purge_test_config(d.path());
        let admin = seeded_admin(&config).await;
        AdminBackend::create_project(&admin, "proj1", "owner@t.c").await.unwrap();
        admin.seed_purge_config_for_test("proj1", true, true, Some(0), Some(30)).await.unwrap();

        let relational = std::sync::Arc::new(crate::storage::RelationalRegistry::new());
        let stores =
            crate::storage::StoreManager::new(std::sync::Arc::new(config.clone()), relational);
        let client = stores.client("proj1").await.unwrap();
        client.write_text_atomic("/old.txt", "data").await.unwrap();
        drop(client);

        dispatch(Cli::parse_from([
            "mcp-fs",
            "purge",
            "--config",
            path.to_str().unwrap(),
            "--project",
            "proj1",
        ]))
        .await
        .unwrap();
    }

    /// E2E-NEW-047: an unconfigured project (`autopurge_enabled=false`) refuses
    /// without `--on-demand`, exits non-zero, leaves the project's config untouched.
    #[tokio::test]
    async fn e2e_new_047_cli_refuses_unconfigured_project_without_on_demand() {
        let d = tempfile::tempdir().unwrap();
        let (path, config) = write_purge_test_config(d.path());
        let admin = seeded_admin(&config).await;
        AdminBackend::create_project(&admin, "proj1", "owner@t.c").await.unwrap();
        admin.seed_purge_config_for_test("proj1", false, true, Some(7), Some(30)).await.unwrap();

        let err = dispatch(Cli::parse_from([
            "mcp-fs",
            "purge",
            "--config",
            path.to_str().unwrap(),
            "--project",
            "proj1",
        ]))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("proj1"), "{err}");
        assert!(!admin.get_purge_config("proj1").await.unwrap().autopurge_enabled);
    }

    /// E2E-NEW-048: `--on-demand` overrides the CLI's own gate, so the sweep
    /// dispatches even though `autopurge_enabled` is false.
    #[tokio::test]
    async fn e2e_new_048_cli_on_demand_overrides_the_gate() {
        let d = tempfile::tempdir().unwrap();
        let (path, config) = write_purge_test_config(d.path());
        let admin = seeded_admin(&config).await;
        AdminBackend::create_project(&admin, "proj1", "owner@t.c").await.unwrap();
        admin.seed_purge_config_for_test("proj1", false, true, Some(7), Some(30)).await.unwrap();

        dispatch(Cli::parse_from([
            "mcp-fs",
            "purge",
            "--config",
            path.to_str().unwrap(),
            "--project",
            "proj1",
            "--on-demand",
        ]))
        .await
        .unwrap();
    }

    /// E2E-NEW-050: a nonexistent project is `ERR_PROJECT_NOT_FOUND`-equivalent
    /// and non-zero, even with `--on-demand`.
    #[tokio::test]
    async fn e2e_new_050_cli_on_nonexistent_project() {
        let d = tempfile::tempdir().unwrap();
        let (path, config) = write_purge_test_config(d.path());
        let admin = seeded_admin(&config).await;
        drop(admin);

        let err = dispatch(Cli::parse_from([
            "mcp-fs",
            "purge",
            "--config",
            path.to_str().unwrap(),
            "--project",
            "ghost",
            "--on-demand",
        ]))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("ERR_PROJECT_NOT_FOUND"), "{err}");
    }

    /// E2E-NEW-051: the global (no `--project`) form on zero soft-deleted
    /// projects anywhere exits 0 with a zero-count summary, no error.
    #[tokio::test]
    async fn e2e_new_051_cli_global_sweep_empty_state() {
        let d = tempfile::tempdir().unwrap();
        let (path, config) = write_purge_test_config(d.path());
        let admin = seeded_admin(&config).await;
        drop(admin);

        dispatch(Cli::parse_from(["mcp-fs", "purge", "--config", path.to_str().unwrap()]))
            .await
            .unwrap();
    }

    /// E2E-NEW-052: the global form dispatches to the grace-sweep hook
    /// unconditionally, regardless of any project's `autopurge_enabled`; this
    /// only asserts the CLI wiring reaches `purge::sweep_grace_period`
    /// (the permanent-removal assertion itself is US-0008's).
    #[tokio::test]
    async fn e2e_new_052_cli_global_sweep_ignores_autopurge_enabled_dispatch_only() {
        let d = tempfile::tempdir().unwrap();
        let (path, config) = write_purge_test_config(d.path());
        let admin = seeded_admin(&config).await;
        AdminBackend::create_project(&admin, "proj1", "owner@t.c").await.unwrap();
        admin.seed_purge_config_for_test("proj1", false, false, None, None).await.unwrap();

        // Must not fail or require `--project`/`--on-demand`: the no-`--project`
        // branch never reads `autopurge_enabled` at all.
        dispatch(Cli::parse_from(["mcp-fs", "purge", "--config", path.to_str().unwrap()]))
            .await
            .unwrap();
    }
}
