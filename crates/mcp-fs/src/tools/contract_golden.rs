//! Test only access to the frozen MCP tool contract at the repo root.
//!
//! The 63 tool names, descriptions and `inputSchema` values are a client and an
//! LLM facing contract. An accidental edit to a description silently changes what
//! an agent is told a tool does, and a reordered schema key changes the bytes a
//! client receives, so both are frozen in `tool-contract-golden.json` and a drift
//! fails a test instead of shipping.
//!
//! This is a snapshot of THIS server, not of anything external: the file is
//! regenerated from the live registry, so the review is the diff. A one line
//! description change shows up as one line; 63 changed tools means something went
//! wrong. Regenerate deliberately with:
//!
//! ```text
//! MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current
//! ```

use crate::mcp::ToolRegistry;
use serde_json::{Map, Value};

/// Set to rewrite the contract from the live registry instead of checking it.
const REWRITE_ENV: &str = "MCPFS_REWRITE_TOOL_CONTRACT";

/// The repo root, not the crate, so a crate only checkout still runs the suite.
pub(crate) const PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../tool-contract-golden.json");

const NOTE: &str = "Frozen MCP tool contract: the 63 tool names, descriptions and inputSchema \
values this server must keep serving. Changing a name, a description or a schema here is a client \
visible contract change, so this file is never hand edited: regenerate it deliberately with the \
command below and review the diff.";

const REGENERATE: &str =
    "MCPFS_REWRITE_TOOL_CONTRACT=1 cargo test -p mcp-fs --lib tool_contract_golden_is_current";

/// The frozen tools, or `None` when the file is absent.
pub(crate) fn frozen_tools() -> Option<Vec<Value>> {
    let raw = std::fs::read_to_string(PATH).ok()?;
    let doc: Value = serde_json::from_str(&raw).expect("the frozen contract must be valid JSON");
    let tools = doc["tools"].as_array().expect("the contract must have a 'tools' array").clone();
    Some(tools)
}

/// Assert every frozen tool selected by `belongs` matches the registry exactly,
/// and that the selection has the expected size.
///
/// The serialized comparison is deliberate: `Value` equality ignores key order,
/// but the order reaches the client, so a reordered property is a real change.
pub(crate) fn assert_family(
    reg: &ToolRegistry,
    belongs: impl Fn(&str) -> bool,
    expected: usize,
    what: &str,
) {
    let Some(frozen) = frozen_tools() else {
        eprintln!("skipped: {PATH} is absent");
        return;
    };

    let mut compared = 0;
    for tool in &frozen {
        let name = tool["name"].as_str().unwrap();
        if !belongs(name) {
            continue;
        }
        compared += 1;
        let mine = reg.resolve(name).unwrap_or_else(|| panic!("{name} is not registered"));
        assert_eq!(
            mine.schema.description,
            tool["description"].as_str().unwrap(),
            "description drift on {name}"
        );
        assert_eq!(mine.schema.input_schema(), tool["inputSchema"], "schema drift on {name}");
        assert_eq!(
            serde_json::to_string(&mine.schema.input_schema()).unwrap(),
            serde_json::to_string(&tool["inputSchema"]).unwrap(),
            "property key order drift on {name}"
        );
    }
    assert_eq!(compared, expected, "the contract must cover all {expected} {what}");
}

/// Every tool the contract covers: the `fs.*`, `admin.*`, `git.*` and `git.auth*`
/// families. The optional families (web, context7, sqlite, db, doc) are config
/// gated, so they are not part of the frozen surface.
fn contract_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    super::register_fs(&mut reg);
    super::admin::register(&mut reg);
    super::git::register(&mut reg);
    super::git_auth::register(&mut reg);
    reg
}

/// Name sorted, so a regeneration diff shows only what actually changed.
fn render(reg: &ToolRegistry) -> String {
    let mut names: Vec<&str> = reg.names().to_vec();
    names.sort_unstable();

    let tools: Vec<Value> = names
        .iter()
        .map(|name| {
            let tool = reg.resolve(name).expect("a listed name resolves");
            let mut entry = Map::new();
            entry.insert("name".into(), Value::String((*name).to_string()));
            entry.insert("description".into(), Value::String(tool.schema.description.clone()));
            entry.insert("inputSchema".into(), tool.schema.input_schema());
            Value::Object(entry)
        })
        .collect();

    let mut doc = Map::new();
    doc.insert("_note".into(), Value::String(NOTE.into()));
    doc.insert("_regenerate".into(), Value::String(REGENERATE.into()));
    doc.insert("tools".into(), Value::Array(tools));

    let mut out = serde_json::to_string_pretty(&Value::Object(doc)).unwrap();
    out.push('\n');
    out
}

/// The contract covers exactly the registry, in both directions.
///
/// The per family tests walk the file and look up each entry, so they cannot see
/// a tool that was ADDED to the registry and never written to the contract. This
/// one compares the whole rendered document, so an addition, a removal and a
/// description edit all fail here.
#[test]
fn tool_contract_golden_is_current() {
    let reg = contract_registry();
    assert_eq!(reg.len(), 63, "the frozen contract covers 35 fs, 10 admin and 18 git tools");
    let rendered = render(&reg);

    if std::env::var_os(REWRITE_ENV).is_some() {
        std::fs::write(PATH, &rendered).expect("the contract must be writable");
        eprintln!("rewrote {PATH} from the live registry: review the diff before committing");
        return;
    }

    let Ok(on_disk) = std::fs::read_to_string(PATH) else {
        eprintln!("skipped: {PATH} is absent");
        return;
    };
    assert_eq!(
        on_disk, rendered,
        "the tool contract drifted: run `{REGENERATE}` and review the diff"
    );
}
