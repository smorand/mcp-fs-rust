//! Command line surface. Port of the C# `Program.Main` verb switch, with the
//! same verbs, the same flags and the same config path resolution.
//!
//! Differences from C#, all cosmetic: clap renders the help text and rejects
//! unknown flags, where the C# rolled its own `OptionValue` scan. The observable
//! behaviour of `serve`, `keys`, `token` and `version` is identical, including
//! the stderr banner and the token going to stdout on its own so it can be piped
//! straight into a file.

use crate::config::ServerConfig;
use crate::keys;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

/// Env var holding a full config path, checked before the dir/name pair.
pub const ENV_CONFIG: &str = "MCP_FS_CONFIG";
/// Env var holding the config directory (default `config`).
pub const ENV_CONFIG_DIR: &str = "MCP_FS_CONFIG_DIR";
/// Env var holding the config file stem (default `local`).
pub const ENV_CONFIG_NAME: &str = "MCP_FS_CONFIG_NAME";

const ABOUT: &str =
    "mcp-fs: filesystem MCP server (SQLite metadata, object store or local blobs)";
const AFTER_HELP: &str = "Config resolution: --config, else MCP_FS_CONFIG, else \
                          MCP_FS_CONFIG_DIR (default 'config') / MCP_FS_CONFIG_NAME \
                          (default 'local').yaml";

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
    match cli.command.unwrap_or(Command::Serve { config: None, git: false, web: false, context7: false, sqlite: false, db: false }) {
        Command::Version => {
            println!("{}", crate::app::VERSION);
            Ok(())
        }
        Command::Keys { dir } => cmd_keys(&dir),
        Command::Token { email, key, issuer, claim, ttl } => {
            cmd_token(&email, key.as_deref(), &issuer, &claim, ttl)
        }
        Command::Serve { config, git, web, context7, sqlite, db } => cmd_serve(config.as_deref(), git, web, context7, sqlite, db).await,
    }
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
    let key_path = key
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(keys::default_private_key_path);
    let token = keys::mint_token_from_file(&key_path, email, issuer, claim, ttl)?;
    // Nothing else on stdout: `mcp-fs token me@x.com > token.txt` must be usable.
    println!("{token}");
    Ok(())
}

async fn cmd_serve(explicit: Option<&std::path::Path>, git: bool, web: bool, context7: bool, sqlite: bool, db: bool) -> anyhow::Result<()> {
    crate::logging::init();
    let resolved = resolve_config_path(explicit);
    let mut config = ServerConfig::load(&resolved)?;
    if git      { config.git.enabled = true; }
    if web      { config.web.enabled = true; }
    if context7 { config.context7.enabled = true; }
    if sqlite   { config.sqlite.enabled = true; }
    if db       { config.db.enabled = true; }
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

/// Config path resolution, mirroring the C# `ConfigLoader.ResolveConfigPath`:
/// `--config`, else `MCP_FS_CONFIG`, else
/// `{MCP_FS_CONFIG_DIR|config}/{MCP_FS_CONFIG_NAME|local}.yaml`.
pub fn resolve_config_path(explicit: Option<&std::path::Path>) -> PathBuf {
    resolve_config_path_with(explicit, |k| std::env::var(k).ok())
}

/// Same resolution with an injectable environment, so it is testable without
/// mutating the process environment (which races across test threads).
pub fn resolve_config_path_with(
    explicit: Option<&std::path::Path>,
    env: impl Fn(&str) -> Option<String>,
) -> PathBuf {
    let non_blank = |v: String| {
        let t = v.trim().to_string();
        (!t.is_empty()).then_some(t)
    };
    if let Some(p) = explicit
        && !p.as_os_str().is_empty()
    {
        return p.to_path_buf();
    }
    if let Some(v) = env(ENV_CONFIG).and_then(non_blank) {
        return PathBuf::from(v);
    }
    let dir = env(ENV_CONFIG_DIR).and_then(non_blank).unwrap_or_else(|| "config".into());
    let name = env(ENV_CONFIG_NAME).and_then(non_blank).unwrap_or_else(|| "local".into());
    PathBuf::from(dir).join(format!("{name}.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn explicit_path_wins_over_everything() {
        let p = resolve_config_path_with(Some(std::path::Path::new("/tmp/x.yaml")), |k| {
            (k == ENV_CONFIG).then(|| "/env.yaml".to_string())
        });
        assert_eq!(p, PathBuf::from("/tmp/x.yaml"));
    }

    #[test]
    fn env_config_wins_over_the_dir_name_pair() {
        let p = resolve_config_path_with(None, |k| match k {
            ENV_CONFIG => Some("/env.yaml".into()),
            ENV_CONFIG_DIR => Some("other".into()),
            _ => None,
        });
        assert_eq!(p, PathBuf::from("/env.yaml"));
    }

    #[test]
    fn dir_and_name_compose_the_default() {
        assert_eq!(resolve_config_path_with(None, no_env), PathBuf::from("config/local.yaml"));
        let p = resolve_config_path_with(None, |k| match k {
            ENV_CONFIG_DIR => Some("/etc/mcpfs".into()),
            ENV_CONFIG_NAME => Some("prod".into()),
            _ => None,
        });
        assert_eq!(p, PathBuf::from("/etc/mcpfs/prod.yaml"));
    }

    #[test]
    fn blank_env_values_fall_back_to_defaults() {
        let p = resolve_config_path_with(None, |k| match k {
            ENV_CONFIG => Some("   ".into()),
            ENV_CONFIG_DIR => Some("".into()),
            ENV_CONFIG_NAME => Some(" ".into()),
            _ => None,
        });
        assert_eq!(p, PathBuf::from("config/local.yaml"));
    }

    #[test]
    fn no_verb_means_serve() {
        let cli = Cli::parse_from(["mcp-fs"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn serve_accepts_short_and_long_config_flags() {
        for args in [
            vec!["mcp-fs", "serve", "--config", "a.yaml"],
            vec!["mcp-fs", "serve", "-c", "a.yaml"],
        ] {
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
            "mcp-fs", "token", "me@test.com", "--key", "/k/jwt.key", "--issuer", "other",
            "--claim", "upn", "--ttl", "60",
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
            dispatch(Cli::parse_from([
                "mcp-fs",
                "serve",
                "--config",
                "/definitely/not/here.yaml",
            ]))
            .await
            .is_err()
        );
    }
}
