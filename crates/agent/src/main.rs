//! An interactive CLI agent driving the mcp-fs tools through an LLM.
//!
//! It is a client, not part of the server: it speaks MCP over HTTP to whatever endpoint
//! the config names, so it exercises the real wire protocol the way any other client
//! would. That is the point, it is the end to end test of the tool surface.

mod config;
mod input;
mod llm;
mod mcp;
mod session;
mod spinner;
mod ui;

use anyhow::{Context, Result, bail};
use clap::Parser;
use config::AgentConfig;
use input::{Input, InputReader};
use llm::{LlmClient, Message};
use session::Session;
use spinner::{Spinner, stop_if_running};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

/// Interactive CLI agent with the mcp-fs tools.
#[derive(Parser)]
#[command(name = "agent", version, about = "Interactive CLI agent with mcp-fs tools")]
struct Cli {
    /// Config file. Defaults to $AGENT_CONFIG, then config/agent_test.yaml.
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Load the token from <tokens_dir>/NAME, overriding mcp.token.
    #[arg(short, long)]
    user: Option<String>,
    /// Resume, or create, a named conversation.
    #[arg(long)]
    conversation: Option<String>,
    /// Print the resolved MCP endpoint and exit.
    ///
    /// This exists so `agent.sh` can find the endpoint without reimplementing the config
    /// resolution and a YAML parse in shell, where both would silently drift.
    #[arg(long)]
    print_mcp_url: bool,
}

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const YELLOW: &str = "\x1b[33m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{RED}[error]{RESET} {}", error_chain(&e));
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(default_config_path);
    let cfg = AgentConfig::load(&config_path)
        .with_context(|| format!("cannot load config, looked for {}", config_path.display()))?;

    // Answered before anything else: no token, no key and no server are needed to report
    // which endpoint the config names.
    if cli.print_mcp_url {
        println!("{}", cfg.mcp.url);
        return Ok(());
    }

    // History lives beside the working directory, so different checkouts stay separate.
    let history_dir = std::env::current_dir()?.join(".agent_history");
    std::fs::create_dir_all(&history_dir)?;

    let mut session = Session::open(&history_dir, cli.conversation.as_deref());
    if cli.conversation.is_some() && !session.is_new() {
        println!("{DIM}  Resuming conversation {YELLOW}{}{RESET}", session.id());
    }
    let mut reader = InputReader::new(&history_dir.join("readline.txt"));

    let token = resolve_token(&cfg, cli.user.as_deref())?;

    println!("{DIM}  Connecting to {}...{RESET}", cfg.mcp.url);
    let client = mcp::McpClient::new(&cfg.mcp.url, &cfg.mcp.auth_header, &token)?;
    let tools = client.list_tools().await.context("MCP connection failed")?;

    let api_key = cfg.api_key().ok_or_else(|| {
        anyhow::anyhow!("LLM API key not set, export ${}", cfg.llm.api_key_env)
    })?;
    let llm = LlmClient::new(&cfg.llm.base_url, &cfg.llm.model, &api_key, cfg.llm.timeout_seconds)?;

    println!(
        "{DIM}  Connected  {GREEN}{} tools{RESET}{DIM}  model={YELLOW}{}{RESET}{DIM}  session={YELLOW}{}{RESET}",
        tools.len(),
        cfg.llm.model,
        session.id()
    );

    // The exact tool names go into the prompt so the model never has to guess whether the
    // separator is a dot or an underscore.
    let catalogue = tools
        .values()
        .map(|t| format!("  - {}: {}", t.name, t.description))
        .collect::<Vec<_>>()
        .join("\n");
    let system_prompt = format!(
        "{}\n\nThe following tools are available (use EXACT names, dots, not underscores):\n{}",
        cfg.system_prompt, catalogue
    );

    let declarations: Vec<serde_json::Value> = tools
        .values()
        .map(|t| llm::tool_declaration(&t.name, &t.description, &t.input_schema))
        .collect();

    let mut history = vec![Message::System(system_prompt.clone())];
    if !session.is_new() {
        history.extend(replay(&session));
    }

    println!(
        "{DIM}  Type {RESET}/help{DIM} for commands, {RESET}/clear{DIM} for a new session, Ctrl+D to quit.{RESET}\n"
    );

    'outer: loop {
        // ── read one prompt, handling commands and continuations ────────────────
        let text = loop {
            let line = match reader.read_input(&format!("\n{CYAN}❯{RESET} "), "  ") {
                Ok(Input::Line(l)) => l,
                Ok(Input::Interrupted) => continue,
                Ok(Input::Eof) => break 'outer,
                Err(e) => {
                    eprintln!("{RED}[error]{RESET} input failed: {e}");
                    break 'outer;
                }
            };

            let trimmed = line.trim();
            match trimmed {
                "exit" | "/exit" => break 'outer,
                "/help" => {
                    print_commands();
                    continue;
                }
                "/history" => {
                    list_sessions(&history_dir);
                    continue;
                }
                "/clear" => {
                    session = Session::open(&history_dir, None);
                    history = vec![Message::System(system_prompt.clone())];
                    print!("\x1b[2J\x1b[H");
                    println!("{DIM}  New session {YELLOW}{}{RESET}\n", session.id());
                    continue;
                }
                _ => {}
            }
            if let Some(id) = trimmed.strip_prefix("/conversation ") {
                let id = id.trim();
                if id.is_empty() {
                    println!("{DIM}  usage: /conversation ID{RESET}");
                    continue;
                }
                session = Session::open(&history_dir, Some(id));
                history = vec![Message::System(system_prompt.clone())];
                if !session.is_new() {
                    history.extend(replay(&session));
                }
                println!("{DIM}  Switched to conversation {YELLOW}{}{RESET}\n", session.id());
                continue;
            }

            if trimmed.is_empty() {
                continue;
            }
            break line.trim().to_string();
        };

        history.push(Message::User(text.clone()));
        let _ = session.append("user", &text);
        println!();

        // ── agentic loop: stream, run tools, repeat until the model stops asking ──
        loop {
            let mut think = Some(Spinner::start("thinking", "36"));
            // A sync handle, because the streaming callback cannot await. Silencing it
            // inside the callback is what keeps frames from landing between tokens.
            let silencer = think.as_ref().map(Spinner::silencer);
            let mut streamed = String::new();
            let mut first_token = true;

            let turn = llm
                .stream_turn(&history, &declarations, |chunk| {
                    if first_token {
                        first_token = false;
                        if let Some(s) = &silencer {
                            s.silence();
                        }
                        println!();
                    }
                    print!("{chunk}");
                    let _ = std::io::stdout().flush();
                    streamed.push_str(chunk);
                })
                .await;

            // Always stop through the helper: assigning None would drop the struct and,
            // before the Drop guard existed, leak the task for the rest of the process.
            stop_if_running(&mut think).await;

            let turn = match turn {
                Ok(t) => t,
                Err(e) => {
                    println!("\n{RED}[error]{RESET} {}", error_chain(&e));
                    // Drop the unanswered user turn so the transcript stays consistent.
                    if matches!(history.last(), Some(Message::User(_))) {
                        history.pop();
                        let _ = session.remove_last();
                    }
                    break;
                }
            };

            // Erase the raw stream and repaint it as markdown. The rewind counts real
            // screen rows, not newlines, so a wrapped line does not leave debris behind.
            //
            // Only a terminal can be rewound. Piped or redirected, the escapes erase
            // nothing and the repaint would simply duplicate the whole answer, so the raw
            // stream that already went out is left as the final text.
            if !streamed.is_empty() && std::io::stdout().is_terminal() {
                let rows = rows_consumed(&streamed, term_cols());
                let mut out = std::io::stdout();
                if rows > 0 {
                    let _ = write!(out, "\x1b[{rows}A");
                }
                let _ = write!(out, "\x1b[1G\x1b[0J");
                let _ = out.flush();
                ui::render_markdown(&streamed);
            } else if !streamed.is_empty() {
                println!();
            }

            if turn.tool_calls.is_empty() {
                if !turn.text.is_empty() {
                    history.push(Message::Assistant(turn.text.clone()));
                    let _ = session.append("assistant", &turn.text);
                }
                println!();
                break;
            }

            history.push(Message::ToolCalls(turn.tool_calls.clone()));

            for call in &turn.tool_calls {
                let args = call.args();
                ui::tool_call(&call.name, &args);

                let mut running = Some(Spinner::start(&format!("running {}", call.name), "33"));
                let outcome = match mcp::resolve_name(&tools, &call.name) {
                    Some(resolved) => client.call_tool(resolved, &args).await.map_err(|e| {
                        // Always keep the full chain on stderr for diagnosis.
                        eprintln!("[tool-error] {}: {e:?}", call.name);
                        e
                    }),
                    None => Err(anyhow::anyhow!(
                        "unknown tool '{}', available: {}",
                        call.name,
                        tools.keys().cloned().collect::<Vec<_>>().join(", ")
                    )),
                };
                stop_if_running(&mut running).await;

                let (text, is_error) = match outcome {
                    Ok(o) => (o.text, o.is_error),
                    Err(e) => (error_chain(&e), true),
                };
                ui::tool_result(&text, is_error);
                if is_error && std::env::var("AGENT_DEBUG").as_deref() == Ok("1") {
                    eprintln!("{text}");
                }
                history.push(Message::ToolResult {
                    call_id: call.id.clone(),
                    content: if text.is_empty() { "(no output)".to_string() } else { text },
                });
            }
        }
    }

    println!("\n{DIM}  Session saved: {YELLOW}{}{RESET}", session.id());
    reader.save();
    Ok(())
}

/// Replay a transcript into conversation messages, dropping anything unrecognised.
fn replay(session: &Session) -> Vec<Message> {
    session
        .load_messages()
        .into_iter()
        .map(|m| match m.role.as_str() {
            "assistant" => Message::Assistant(m.content),
            _ => Message::User(m.content),
        })
        .collect()
}

/// Resolve the bearer token: `--user` wins, then the config value.
fn resolve_token(cfg: &AgentConfig, user: Option<&str>) -> Result<String> {
    let Some(user) = user else {
        return Ok(cfg.mcp.token.clone());
    };
    let dir = Path::new(&cfg.mcp.tokens_dir);
    let dir = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()?.join(dir)
    };
    let file = dir.join(user);
    if !file.exists() {
        bail!(
            "token file not found: {}\n  create it with: mcp-fs token {user}@example.com --key .keys/jwt.key > {}",
            file.display(),
            file.display()
        );
    }
    let token = std::fs::read_to_string(&file)
        .with_context(|| format!("reading {}", file.display()))?
        .trim()
        .to_string();
    if token.is_empty() {
        bail!("token file is empty: {}", file.display());
    }
    println!("{DIM}  User: {YELLOW}{user}{RESET}{DIM}  (token from {}){RESET}", file.display());
    Ok(token)
}

/// Where to look for the config when `--config` is absent.
fn default_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("AGENT_CONFIG")
        && !p.trim().is_empty()
    {
        return PathBuf::from(p);
    }
    PathBuf::from("config/agent_test.yaml")
}

/// Terminal width, floored so the arithmetic stays sane.
fn term_cols() -> usize {
    crossterm::terminal::size().map(|(c, _)| c as usize).unwrap_or(80).max(20)
}

/// How many screen rows a block of text occupies when printed from column 0.
///
/// Counting newlines alone is not enough: a single logical line longer than the terminal
/// wraps onto several rows, and the cursor rewind has to account for every one of them.
fn rows_consumed(text: &str, cols: usize) -> usize {
    use unicode_width::UnicodeWidthStr;
    let mut rows = 0usize;
    let lines: Vec<&str> = text.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        rows += line.width() / cols;
        if i + 1 < lines.len() {
            rows += 1;
        }
    }
    rows
}

/// Flatten an error and its causes into a message worth showing a model.
///
/// The model has to act on the failure, so the root cause matters more than the outermost
/// wrapper. Duplicate messages are dropped, which anyhow chains often contain.
fn error_chain(e: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for cause in e.chain() {
        let msg = cause.to_string().trim().to_string();
        if !msg.is_empty() && !parts.contains(&msg) {
            parts.push(msg);
        }
    }
    match parts.len() {
        0 => "error: unknown failure".to_string(),
        1 => format!("error: {}", parts[0]),
        _ => format!("error: {}\n  caused by: {}", parts[0], parts[1..].join("\n  caused by: ")),
    }
}

fn print_commands() {
    println!("Commands:");
    println!("  {CYAN}/clear{RESET}                    Start a new session");
    println!("  {CYAN}/history{RESET}                  List saved conversations");
    println!("  {CYAN}/conversation{RESET} {YELLOW}ID{RESET}        Switch to a conversation");
    println!("  {CYAN}/help{RESET}                     Show this help");
    println!("  {CYAN}exit{RESET} or Ctrl+D            Quit");
    println!("  Append a backslash to a line for multi line input");
    println!();
}

fn list_sessions(history_dir: &Path) {
    let rows = session::list_sessions(history_dir, 20);
    if rows.is_empty() {
        println!("{DIM}  no saved conversations yet{RESET}");
        return;
    }
    println!("  {DIM}ID                Date              Messages{RESET}");
    for (id, modified, count) in rows {
        let when: chrono::DateTime<chrono::Local> = modified.into();
        println!("  {id:<17} {}  {count}", when.format("%Y-%m-%d %H:%M"));
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_consumed_counts_newlines() {
        assert_eq!(rows_consumed("one", 80), 0, "a short single line stays on its row");
        assert_eq!(rows_consumed("one\ntwo", 80), 1);
        assert_eq!(rows_consumed("a\nb\nc", 80), 2);
    }

    #[test]
    fn rows_consumed_counts_wrapped_rows_which_newline_counting_misses() {
        // 200 columns of text in an 80 column terminal is two extra rows.
        let long = "x".repeat(200);
        assert_eq!(rows_consumed(&long, 80), 2);
        // And wrapping combines with newlines.
        assert_eq!(rows_consumed(&format!("{long}\nshort"), 80), 3);
    }

    #[test]
    fn rows_consumed_handles_an_exact_multiple_of_the_width() {
        assert_eq!(rows_consumed(&"x".repeat(80), 80), 1);
        assert_eq!(rows_consumed(&"x".repeat(79), 80), 0);
    }

    #[test]
    fn rows_consumed_measures_wide_characters_by_display_width() {
        // 50 CJK characters are 100 columns, so they wrap once in an 80 column terminal.
        assert_eq!(rows_consumed(&"漢".repeat(50), 80), 1);
    }

    #[test]
    fn rows_consumed_on_empty_text_is_zero() {
        assert_eq!(rows_consumed("", 80), 0);
    }

    #[test]
    fn error_chain_reports_a_single_message_plainly() {
        let e = anyhow::anyhow!("it broke");
        assert_eq!(error_chain(&e), "error: it broke");
    }

    #[test]
    fn error_chain_unwraps_causes_in_order() {
        let root = anyhow::anyhow!("connection refused");
        let wrapped = root.context("POST failed").context("MCP connection failed");
        let out = error_chain(&wrapped);
        assert_eq!(
            out,
            "error: MCP connection failed\n  caused by: POST failed\n  caused by: connection refused"
        );
    }

    #[test]
    fn error_chain_drops_a_duplicated_message() {
        let e = anyhow::anyhow!("same").context("same");
        assert_eq!(error_chain(&e), "error: same", "the repeat is not shown twice");
    }

    #[test]
    fn the_default_config_path_prefers_the_environment() {
        let var = "AGENT_CONFIG";
        let before = std::env::var(var).ok();
        // SAFETY: this test does not run concurrently with another reader of this name.
        unsafe { std::env::set_var(var, "/tmp/custom.yaml") };
        assert_eq!(default_config_path(), PathBuf::from("/tmp/custom.yaml"));
        unsafe { std::env::set_var(var, "  ") };
        assert_eq!(
            default_config_path(),
            PathBuf::from("config/agent_test.yaml"),
            "a blank value falls back"
        );
        match before {
            Some(v) => unsafe { std::env::set_var(var, v) },
            None => unsafe { std::env::remove_var(var) },
        }
    }

    #[test]
    fn resolve_token_without_a_user_uses_the_config_value() {
        let mut cfg = AgentConfig::default();
        cfg.mcp.token = "from-config".into();
        assert_eq!(resolve_token(&cfg, None).unwrap(), "from-config");
    }

    #[test]
    fn resolve_token_reads_and_trims_the_user_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("alice"), "  eyJtoken  \n").unwrap();
        let mut cfg = AgentConfig::default();
        cfg.mcp.tokens_dir = d.path().display().to_string();
        assert_eq!(resolve_token(&cfg, Some("alice")).unwrap(), "eyJtoken");
    }

    #[test]
    fn a_missing_token_file_explains_how_to_create_it() {
        let d = tempfile::tempdir().unwrap();
        let mut cfg = AgentConfig::default();
        cfg.mcp.tokens_dir = d.path().display().to_string();
        let e = resolve_token(&cfg, Some("bob")).unwrap_err().to_string();
        assert!(e.contains("token file not found"), "got {e}");
        assert!(e.contains("mcp-fs token"), "the message must show the fix: {e}");
    }

    #[test]
    fn an_empty_token_file_is_refused_rather_than_sent_as_a_blank_bearer() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("carol"), "\n  \n").unwrap();
        let mut cfg = AgentConfig::default();
        cfg.mcp.tokens_dir = d.path().display().to_string();
        let e = resolve_token(&cfg, Some("carol")).unwrap_err().to_string();
        assert!(e.contains("empty"), "got {e}");
    }
}
