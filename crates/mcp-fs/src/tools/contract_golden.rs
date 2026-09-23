//! Test only access to the frozen MCP tool contract at the repo root.
//!
//! The 94 tool names, descriptions and `inputSchema` values are a client and an
//! LLM facing contract. An accidental edit to a description silently changes what
//! an agent is told a tool does, and a reordered schema key changes the bytes a
//! client receives, so both are frozen in `tool-contract-golden.json` and a drift
//! fails a test instead of shipping.
//!
//! This is a snapshot of THIS server, not of anything external: the file is
//! regenerated from the live registry, so the review is the diff. A one line
//! description change shows up as one line; 94 changed tools means something went
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

const NOTE: &str = "Frozen MCP tool contract: the 94 tool names, descriptions and inputSchema \
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
    super::git_pr::register(&mut reg);
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
    assert_eq!(reg.len(), 94, "the frozen contract covers 35 fs, 10 admin and 49 git tools");
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

/// The 18 git family tools that existed before the full git dev process
/// specification. Literal, because deriving them from the registry would make
/// the enumeration below compare a set against itself.
const PRE_SPEC_GIT_TOOLS: [&str; 18] = [
    "git.auth",
    "git.auth_revoke",
    "git.auth_status",
    "git.blame",
    "git.branches",
    "git.checkout_file",
    "git.commit",
    "git.diff",
    "git.init",
    "git.log",
    "git.remote_clone",
    "git.remote_fetch",
    "git.remote_pull",
    "git.remote_push",
    "git.show",
    "git.status",
    "git.tags",
    "git.token_set",
];

/// The 31 tools FR-NEW-349 enumerates, verbatim and in its order. This literal
/// is the authority: a tool registered but not listed here, or listed and never
/// registered, fails `e2e_new_960_*`.
const NEW_GIT_TOOLS: [&str; 31] = [
    "git.branch_create",
    "git.branch_switch",
    "git.branch_delete",
    "git.branch_reset",
    "git.stash_save",
    "git.stash_list",
    "git.stash_apply",
    "git.stash_pop",
    "git.stash_drop",
    "git.remote_add",
    "git.remote_remove",
    "git.remote_list",
    "git.merge",
    "git.merge_resolve",
    "git.merge_abort",
    "git.rebase",
    "git.rebase_continue",
    "git.rebase_abort",
    "git.cherry_pick",
    "git.cherry_pick_continue",
    "git.cherry_pick_abort",
    "git.reset",
    "git.revert",
    "git.revert_continue",
    "git.revert_abort",
    "git.pr_create",
    "git.pr_list",
    "git.pr_get",
    "git.pr_diff",
    "git.pr_merge",
    "git.pr_review",
];

/// Compare a registered git family against `PRE_SPEC_GIT_TOOLS + NEW_GIT_TOOLS`.
///
/// Returns the failure text instead of panicking so the negative test can prove
/// the gate actually fires, which an `assert!` inside the happy test could not.
fn git_enumeration_verdict(registered: &[&str]) -> Result<(), String> {
    let expected: std::collections::BTreeSet<&str> =
        PRE_SPEC_GIT_TOOLS.iter().chain(NEW_GIT_TOOLS.iter()).copied().collect();
    let actual: std::collections::BTreeSet<&str> =
        registered.iter().copied().filter(|n| n.starts_with("git.")).collect();

    let extra: Vec<&str> = actual.difference(&expected).copied().collect();
    let missing: Vec<&str> = expected.difference(&actual).copied().collect();
    if extra.is_empty() && missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "registered but not enumerated: {extra:?}; enumerated but not registered: {missing:?}"
    ))
}

/// E2E-NEW-960: the enumerated names are exactly the registered git family.
#[test]
fn e2e_new_960_the_thirty_one_enumerated_names_are_exactly_the_new_tool_set() {
    let reg = contract_registry();
    git_enumeration_verdict(reg.names()).expect("the git family must match the enumeration");

    assert_eq!(PRE_SPEC_GIT_TOOLS.len() + NEW_GIT_TOOLS.len(), 49);
    assert_eq!(reg.len(), 94);

    // Every new tool is frozen, and carries the family's required mount_id.
    let Some(frozen) = frozen_tools() else {
        eprintln!("skipped: {PATH} is absent");
        return;
    };
    for name in NEW_GIT_TOOLS {
        let entry = frozen
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} is missing from the frozen contract"));
        assert_eq!(
            entry["inputSchema"]["properties"]["mount_id"]["type"], "string",
            "{name} must take a string mount_id"
        );
        assert!(
            entry["inputSchema"]["required"]
                .as_array()
                .is_some_and(|r| r.iter().any(|v| v == "mount_id")),
            "{name} must require mount_id"
        );
    }
}

/// E2E-NEW-961: an unlisted `git.*` tool fails the enumeration, by name.
#[test]
fn e2e_new_961_registering_an_unlisted_git_tool_fails_the_enumeration() {
    let mut reg = contract_registry();
    reg.add(
        crate::mcp::ToolSchema::new("git.rogue", "A tool nobody enumerated."),
        crate::mcp::registry::handler(|_ctx, _args| async move {
            Err(crate::errors::ToolError::not_supported("rogue"))
        }),
    );

    let err = git_enumeration_verdict(reg.names())
        .expect_err("a tool outside the enumeration must fail the gate");
    assert!(err.contains("git.rogue"), "the failure must name the offender: {err}");
}

/// E2E-NEW-931 / E2E-MOD-405 / E2E-MOD-407 / E2E-MOD-408: every hardcoded count
/// site agrees with the live registry.
///
/// The five sites that move when a git tool is added are recomputed here rather
/// than restated, so a divergence between two of them fails. `all.rs`'s
/// admin-only (10) and git-disabled (52) assertions are deliberately absent:
/// neither moves with the git family.
#[test]
fn e2e_new_931_every_hardcoded_tool_count_site_reports_the_new_numbers() {
    let reg = contract_registry();
    let git_family = reg.names().iter().filter(|n| n.starts_with("git.")).count();
    let fs_family = reg.names().iter().filter(|n| n.starts_with("fs.")).count();
    let admin_family = reg.names().iter().filter(|n| n.starts_with("admin.")).count();

    // contract_golden.rs's own total, and the subtotals all.rs asserts.
    assert_eq!(reg.len(), 94);
    assert_eq!(git_family, 49);
    assert_eq!(fs_family + admin_family + git_family, reg.len());

    // git.rs's ALL_GIT_TOOLS covers the family minus the four auth tools and the
    // six pull request tools, which register from their own entry points.
    let auth = reg.names().iter().filter(|n| n.starts_with("git.auth") || **n == "git.token_set");
    assert_eq!(auth.count(), 4);
    assert_eq!(reg.names().iter().filter(|n| n.starts_with("git.pr_")).count(), 6);

    let Some(frozen) = frozen_tools() else {
        eprintln!("skipped: {PATH} is absent");
        return;
    };
    assert_eq!(frozen.len(), 94, "the golden file holds one entry per registered tool");

    // The contract text documents the same names, set for set.
    let text =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../TOOL_CONTRACT.txt"))
            .expect("TOOL_CONTRACT.txt must be readable");
    for name in reg.names() {
        assert!(text.contains(name), "{name} is undocumented in TOOL_CONTRACT.txt");
    }
}

/// E2E-NEW-692: every new name resolves, including its underscore form.
#[test]
fn e2e_new_692_every_new_git_tool_resolves_in_both_spellings() {
    let reg = contract_registry();
    for name in NEW_GIT_TOOLS {
        assert!(reg.resolve(name).is_some(), "{name} is not registered");
        let underscored = name.replace('.', "_");
        assert!(reg.resolve(&underscored).is_some(), "{underscored} must resolve too");
    }
}

/// E2E-NEW-693 / E2E-NEW-799: the pull request family is frozen with the exact
/// enumerations clients branch on.
#[test]
fn e2e_new_799_the_pull_request_family_is_frozen_with_its_enumerations() {
    let reg = contract_registry();
    let schema_of = |name: &str| reg.resolve(name).expect("registered").schema.input_schema();

    // The frozen wording joins the last value with "or", so the assertion is on
    // the values named, not on a punctuation the contract already froze.
    let names_all = |tool: &str, param: &str, values: &[&str]| {
        let text = schema_of(tool)["properties"][param]["description"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        for v in values {
            assert!(text.contains(v), "{tool}.{param} must name {v}: {text}");
        }
    };
    names_all("git.pr_list", "state", &["open", "closed", "merged", "all"]);
    names_all("git.pr_merge", "strategy", &["merge", "squash", "rebase"]);
    names_all("git.pr_review", "verdict", &["approve", "request_changes", "comment"]);
}

/// E2E-NEW-810: `on_conflict` was deleted from `git.remote_pull` and must not
/// reappear, neither in the registered schema nor in the frozen contract.
#[test]
fn e2e_new_810_on_conflict_is_absent_from_the_frozen_schema() {
    let reg = contract_registry();
    let schema = reg.resolve("git.remote_pull").expect("registered").schema.input_schema();
    let keys: Vec<&str> =
        schema["properties"].as_object().expect("an object").keys().map(String::as_str).collect();
    assert_eq!(keys, ["mount_id", "branch", "remote"], "unexpected git.remote_pull parameters");

    let text =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../TOOL_CONTRACT.txt"))
            .expect("TOOL_CONTRACT.txt must be readable");
    assert!(!text.contains("on_conflict"), "on_conflict must be gone from the contract");
}

// ── documentation parity (US-031) ───────────────────────────────────────────

/// The two documents whose tool surface must match the registry.
const AGENTS_MD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../AGENTS.md");
const TOOLS_MD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.agent_docs/tools.md");

/// The integer immediately preceding the first occurrence of `marker`.
///
/// Documentation counts are written inline ("94 tools", "39 `git.*`"), so the
/// parse is "the digits that run backwards from the marker" rather than a
/// regex, which would need a dependency this project does not carry.
fn int_before(text: &str, marker: &str) -> usize {
    let at = text.find(marker).unwrap_or_else(|| panic!("'{marker}' is absent from the document"));
    let head = text[..at].trim_end();
    let digits: String = head
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    digits.parse().unwrap_or_else(|_| panic!("no integer precedes '{marker}': ...{head:?}"))
}

/// Every `git.*` tool named in the first cell of a markdown table row.
///
/// Only that position counts as "documented": prose mentions `git.hosts` and
/// `git.enabled`, which are configuration keys and not tools, so a looser scan
/// would report names that can never be registered.
fn documented_git_tools(text: &str) -> std::collections::BTreeSet<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("| `"))
        .filter_map(|rest| rest.split('`').next())
        .filter(|name| name.starts_with("git."))
        .map(str::to_string)
        .collect()
}

/// Returns the failure text instead of panicking, so the negative test can prove
/// the guard fires rather than passing vacuously.
fn documentation_parity_verdict(
    registered: &[&str],
    doc: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let registered: std::collections::BTreeSet<String> =
        registered.iter().filter(|n| n.starts_with("git.")).map(|n| (*n).to_string()).collect();
    let undocumented: Vec<&String> = registered.difference(doc).collect();
    let phantom: Vec<&String> = doc.difference(&registered).collect();
    if undocumented.is_empty() && phantom.is_empty() {
        return Ok(());
    }
    Err(format!(
        "registered but undocumented: {undocumented:?}; documented but not registered: {phantom:?}"
    ))
}

/// E2E-NEW-957: the documented tool counts are the live registry's counts.
#[test]
fn e2e_new_957_the_documented_tool_count_matches_the_live_registry() {
    let reg = contract_registry();
    let git_family = reg.names().iter().filter(|n| n.starts_with("git.")).count();

    let agents = std::fs::read_to_string(AGENTS_MD).expect("AGENTS.md must be readable");
    let tools = std::fs::read_to_string(TOOLS_MD).expect(".agent_docs/tools.md must be readable");

    assert_eq!(int_before(&agents, " tools:"), reg.len(), "AGENTS.md total");
    assert_eq!(int_before(&tools, " tools)"), reg.len(), ".agent_docs/tools.md title total");
    assert_eq!(
        int_before(&agents, " `git.*`")
            + int_before(&agents, " `git.auth*`")
            + int_before(&agents, " `git.pr_*`"),
        git_family,
        "AGENTS.md git subtotals"
    );
    // The three `## git*` section headers of the reference, which must add up to
    // the same family: a header left behind is as wrong as a stale total.
    let sections: Vec<usize> = tools
        .lines()
        .filter(|l| l.starts_with("## git"))
        .map(|l| int_before(l, ", registered only when `git.enabled`"))
        .collect();
    assert_eq!(sections.len(), 3, ".agent_docs/tools.md must keep one section per git family");
    assert_eq!(
        sections.iter().sum::<usize>(),
        git_family,
        ".agent_docs/tools.md git section headers"
    );
    // The index line naming the reference must move with the reference itself.
    assert_eq!(int_before(&agents, " tool reference"), reg.len(), "AGENTS.md documentation index");
    assert_eq!(int_before(&agents, " tool schemas"), reg.len(), "AGENTS.md contract line");
}

/// E2E-NEW-958: `.agent_docs/tools.md` documents exactly the registered git family.
#[test]
fn e2e_new_958_the_documentation_lists_every_registered_git_tool() {
    let reg = contract_registry();
    let tools = std::fs::read_to_string(TOOLS_MD).expect(".agent_docs/tools.md must be readable");
    documentation_parity_verdict(reg.names(), &documented_git_tools(&tools))
        .expect("the documented git surface must be the registered one");
}

/// E2E-NEW-959: a tool missing from the documentation fails that parity test, by
/// name. Without this the guard could pass vacuously.
#[test]
fn e2e_new_959_a_tool_missing_from_the_documentation_fails_the_parity_test() {
    let mut reg = contract_registry();
    reg.add(
        crate::mcp::ToolSchema::new("git.undocumented_probe", "A tool nobody documented."),
        crate::mcp::registry::handler(|_ctx, _args| async move {
            Err(crate::errors::ToolError::not_supported("probe"))
        }),
    );

    let tools = std::fs::read_to_string(TOOLS_MD).expect(".agent_docs/tools.md must be readable");
    let err = documentation_parity_verdict(reg.names(), &documented_git_tools(&tools))
        .expect_err("an undocumented tool must fail the gate");
    assert!(err.contains("git.undocumented_probe"), "the failure must name the offender: {err}");
}
