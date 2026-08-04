//! Differential parity harness: the objective judge of 1:1 parity between the
//! C# reference server and the Rust port.
//!
//! Two modes:
//!   capture  hit one server, write every response to a golden JSON file
//!   compare  hit the other server, diff against the golden file
//!
//! Running them separately means the two servers never have to be up at the same
//! time, and the golden file can be committed as a regression baseline.
//!
//! Volatile values (timestamps, version, host paths) are normalized away, and an
//! error text is reduced to `tool + ERR_* code` so a differently worded message
//! does not fail parity while a wrong code does. See `normalize.rs`.

mod corpus;
mod normalize;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use normalize::Options;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "parity-harness", about = "Differential parity tester for mcp-fs")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Replay the corpus against a server and write a golden file.
    Capture {
        #[arg(long, default_value = "http://127.0.0.1:5002")]
        base: String,
        #[arg(long)]
        token: String,
        /// Project to provision and exercise. Defaults to a fresh id per run so the
        /// replay never inherits state from a previous capture.
        #[arg(long)]
        project: Option<String>,
        /// Owner used when provisioning the project.
        #[arg(long)]
        owner: String,
        #[arg(long, default_value = "parity-golden.json")]
        out: PathBuf,
    },
    /// Replay the corpus against a server and diff against a golden file.
    Compare {
        #[arg(long, default_value = "http://127.0.0.1:5003")]
        base: String,
        #[arg(long)]
        token: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        owner: String,
        #[arg(long, default_value = "parity-golden.json")]
        golden: PathBuf,
        /// Blank out free-form message fields as well as error sentences.
        #[arg(long)]
        relax_messages: bool,
    },
}

#[derive(Serialize, Deserialize)]
struct Golden {
    base: String,
    project: String,
    /// label -> normalized response
    steps: BTreeMap<String, Recorded>,
}

#[derive(Serialize, Deserialize, PartialEq)]
struct Recorded {
    status: u16,
    /// Present for JSON responses; `None` when the body was not JSON.
    body: Option<Value>,
    /// Raw body, kept only when it was not JSON (so we can still diff it).
    raw: Option<String>,
}

struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
    mcp_path: String,
}

impl Client {
    fn new(base: &str, token: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("http client"),
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            mcp_path: "/mcp".to_string(),
        }
    }

    /// Send one JSON-RPC message and return (status, parsed body).
    /// Handles both SSE framing (`event: message\ndata: {...}`) and plain JSON.
    async fn mcp(&self, id: u64, method: &str, params: &Value) -> Result<(u16, Option<Value>, String)> {
        let payload = if params.is_null() || params == &json!({}) {
            json!({"jsonrpc":"2.0","id":id,"method":method})
        } else {
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        };
        let r = self
            .http
            .post(format!("{}{}", self.base, self.mcp_path))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("X-Forwarded-Authorization", format!("Bearer {}", self.token))
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("mcp {method}"))?;
        let status = r.status().as_u16();
        let text = r.text().await.unwrap_or_default();
        Ok((status, parse_sse_or_json(&text), text))
    }

    async fn tool(&self, id: u64, name: &str, args: &Value) -> Result<(u16, Option<Value>, String)> {
        self.mcp(id, "tools/call", &json!({"name": name, "arguments": args})).await
    }

    async fn rest(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Option<Value>, String)> {
        let url = format!("{}{}", self.base, path);
        let mut req = match method {
            "GET" => self.http.get(url),
            "POST" => self.http.post(url),
            other => bail!("unsupported REST method {other}"),
        }
        .header("Authorization", format!("Bearer {}", self.token));
        if let Some(b) = body {
            req = req.json(b);
        }
        let r = req.send().await.with_context(|| format!("rest {method} {path}"))?;
        let status = r.status().as_u16();
        let text = r.text().await.unwrap_or_default();
        Ok((status, serde_json::from_str(&text).ok(), text))
    }

    async fn public(&self, path: &str) -> Result<(u16, Option<Value>, String)> {
        let r = self
            .http
            .get(format!("{}{}", self.base, path))
            .send()
            .await
            .with_context(|| format!("public {path}"))?;
        let status = r.status().as_u16();
        let text = r.text().await.unwrap_or_default();
        Ok((status, serde_json::from_str(&text).ok(), text))
    }
}

/// A fresh project id per run. Cleaning up a project is not enough on the reference
/// implementation (tearing a project down leaves its volume behind), so a replay that
/// reused one id would hit ERR_NO_CLOBBER on every write the second time around.
fn fresh_project() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("parity-{:x}", n & 0xffff_ffff)
}

/// Extract the JSON payload from an SSE frame, or parse the body as plain JSON.
fn parse_sse_or_json(text: &str) -> Option<Value> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data: ")
            && let Ok(v) = serde_json::from_str::<Value>(rest.trim())
        {
            return Some(v);
        }
    }
    serde_json::from_str(text).ok()
}

/// Provision the probe project so every corpus step has something to work on.
/// Failures are tolerated: a rerun finds the project already there.
async fn provision(c: &Client, project: &str, owner: &str) -> Result<()> {
    let _ = c
        .tool(9_000, "admin.create_project", &json!({"project_id": project, "owner": owner}))
        .await?;
    let _ = c
        .tool(9_001, "admin.add_member", &json!({"project_id": project, "person": owner}))
        .await?;
    Ok(())
}

/// Best-effort cleanup so repeated runs start from the same state.
async fn cleanup(c: &Client, project: &str) -> Result<()> {
    let _ = c.tool(9_900, "admin.delete_project", &json!({"project_id": project})).await?;
    Ok(())
}

async fn replay(c: &Client, project: &str, owner: &str, opts: &Options) -> Result<BTreeMap<String, Recorded>> {
    // Start from a clean slate so the corpus is deterministic.
    cleanup(c, project).await?;
    provision(c, project, owner).await?;

    let mut out = BTreeMap::new();
    for (i, step) in corpus::build(project).into_iter().enumerate() {
        let id = 1_000 + i as u64;
        let (status, body, raw) = match &step {
            corpus::Step::Mcp { method, params, .. } => c.mcp(id, method, params).await?,
            corpus::Step::Rest { method, path, body, .. } => {
                c.rest(method, path, body.as_ref()).await?
            }
            corpus::Step::Public { path, .. } => c.public(path).await?,
        };
        let normalized = body.as_ref().map(|b| {
            let mut n = match &step {
                corpus::Step::Mcp { .. } => normalize::normalize_rpc(b, opts),
                _ => normalize::normalize(b, opts),
            };
            // Drop host-environment noise and unspecified ordering for the few steps
            // where it would report noise instead of a behavioural difference.
            normalize::tame_environment(step.label(), &mut n, project);
            // The project id is per run, so mask it everywhere it surfaces.
            normalize::mask_project(&mut n, project);
            n
        });
        out.insert(
            step.label().to_string(),
            Recorded {
                status,
                body: normalized,
                raw: if body.is_none() { Some(truncate(&raw)) } else { None },
            },
        );
    }
    Ok(out)
}

fn truncate(s: &str) -> String {
    if s.len() > 400 { format!("{}…", &s[..400]) } else { s.to_string() }
}

/// Print the first difference inside two values, to keep the report readable.
fn first_diff(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, xv) in x {
                match y.get(k) {
                    None => return Some(format!("{path}.{k}: only in golden")),
                    Some(yv) => {
                        if let Some(d) = first_diff(xv, yv, &format!("{path}.{k}")) {
                            return Some(d);
                        }
                    }
                }
            }
            for k in y.keys() {
                if !x.contains_key(k) {
                    return Some(format!("{path}.{k}: only in candidate"));
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: array len {} vs {}", x.len(), y.len()));
            }
            for (i, (xv, yv)) in x.iter().zip(y).enumerate() {
                if let Some(d) = first_diff(xv, yv, &format!("{path}[{i}]")) {
                    return Some(d);
                }
            }
            None
        }
        _ if a == b => None,
        _ => Some(format!(
            "{path}: {} vs {}",
            compact(a),
            compact(b)
        )),
    }
}

fn compact(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_default();
    if s.len() > 160 { format!("{}…", &s[..160]) } else { s }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Capture { base, token, project, owner, out } => {
            let project = project.unwrap_or_else(fresh_project);
            let c = Client::new(&base, &token);
            let opts = Options::default();
            let steps = replay(&c, &project, &owner, &opts).await?;
            let g = Golden { base: base.clone(), project: project.clone(), steps };
            std::fs::write(&out, serde_json::to_string_pretty(&g)?)?;
            println!(
                "captured {} steps from {} into {}",
                g.steps.len(),
                base,
                out.display()
            );
            Ok(())
        }
        Cmd::Compare { base, token, project, owner, golden, relax_messages } => {
            let project = project.unwrap_or_else(fresh_project);
            let raw = std::fs::read_to_string(&golden)
                .with_context(|| format!("reading golden file {}", golden.display()))?;
            let g: Golden = serde_json::from_str(&raw)?;
            let c = Client::new(&base, &token);
            let opts = Options { relax_messages, ..Default::default() };
            let mine = replay(&c, &project, &owner, &opts).await?;

            let mut mismatches = Vec::new();
            let mut missing = Vec::new();
            let mut checked = 0usize;

            for (label, expected) in &g.steps {
                match mine.get(label) {
                    None => missing.push(label.clone()),
                    Some(actual) => {
                        checked += 1;
                        if actual.status != expected.status {
                            mismatches.push(format!(
                                "{label}: HTTP {} vs {} (golden)",
                                actual.status, expected.status
                            ));
                            continue;
                        }
                        match (&expected.body, &actual.body) {
                            (Some(e), Some(a)) => {
                                if let Some(d) = first_diff(e, a, "") {
                                    mismatches.push(format!("{label}: {d}"));
                                }
                            }
                            (None, None) => {}
                            (Some(_), None) => {
                                mismatches.push(format!("{label}: candidate body is not JSON"))
                            }
                            (None, Some(_)) => {
                                mismatches.push(format!("{label}: golden body was not JSON"))
                            }
                        }
                    }
                }
            }

            println!("compared {checked} steps against {}", golden.display());
            if !missing.is_empty() {
                println!("\nmissing from candidate ({}):", missing.len());
                for m in &missing {
                    println!("  {m}");
                }
            }
            if mismatches.is_empty() && missing.is_empty() {
                println!("\nPARITY OK: no differences");
                return Ok(());
            }
            println!("\ndifferences ({}):", mismatches.len());
            for m in &mismatches {
                println!("  {m}");
            }
            bail!("{} difference(s), {} missing", mismatches.len(), missing.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_frame_is_parsed() {
        let text = "event: message\ndata: {\"result\":{\"x\":1},\"id\":1,\"jsonrpc\":\"2.0\"}\n\n";
        let v = parse_sse_or_json(text).expect("parsed");
        assert_eq!(v["result"]["x"], 1);
        assert_eq!(v["id"], 1);
    }

    #[test]
    fn plain_json_is_parsed() {
        let v = parse_sse_or_json(r#"{"status":"ok"}"#).expect("parsed");
        assert_eq!(v["status"], "ok");
    }

    #[test]
    fn non_json_yields_none() {
        assert!(parse_sse_or_json("Unexpected message, expect initialize request").is_none());
    }

    #[test]
    fn identical_values_have_no_diff() {
        let a = json!({"a":1,"b":[1,2],"c":{"d":"x"}});
        assert_eq!(first_diff(&a, &a.clone(), ""), None);
    }

    #[test]
    fn scalar_difference_is_reported_with_a_path() {
        let a = json!({"outer":{"inner":1}});
        let b = json!({"outer":{"inner":2}});
        let d = first_diff(&a, &b, "").expect("a diff");
        assert!(d.starts_with(".outer.inner:"), "got {d}");
    }

    #[test]
    fn missing_and_extra_keys_are_reported() {
        let a = json!({"x":1});
        let b = json!({"y":1});
        let d = first_diff(&a, &b, "").expect("a diff");
        assert!(d.contains("only in golden"), "got {d}");
        let d2 = first_diff(&json!({}), &json!({"z":1}), "").expect("a diff");
        assert!(d2.contains("only in candidate"), "got {d2}");
    }

    #[test]
    fn array_length_difference_is_reported() {
        let d = first_diff(&json!([1, 2]), &json!([1]), "").expect("a diff");
        assert!(d.contains("array len 2 vs 1"), "got {d}");
    }

    #[test]
    fn recorded_equality_is_structural() {
        let a = Recorded { status: 200, body: Some(json!({"a":1})), raw: None };
        let b = Recorded { status: 200, body: Some(json!({"a":1})), raw: None };
        assert!(a == b);
    }
}
