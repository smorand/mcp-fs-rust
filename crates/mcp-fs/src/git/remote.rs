//! The `git.hosts` map: sole owner of interpreting which git hosts this server
//! trusts and what credential policy each carries.
//!
//! `config.rs` declares the `hosts` field on [`crate::config::GitConfig`] and
//! calls [`validate_hosts`] once from `ServerConfig::validate`. Every other read
//! of an entry's value, including resolution, lives here. This module does not
//! resolve a git remote URL to a hostname yet (that lands with the URL parsing
//! this module already exposes via the `url` crate for its own key validation);
//! wiring a clone's URL through to [`resolve_host`] is a later story.
//!
//! Host matching is exact only (DEC-015): no wildcard, no substring, no prefix
//! fallback. A host absent from the map is not a `Provider::Anonymous`, it is a
//! [`Result::Err`], because silently downgrading an undeclared host to anonymous
//! would be exactly the ambiguity exact matching exists to remove.

use std::collections::HashMap;
use std::fmt;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Deserializer, Serialize};

use crate::config::GitConfig;
use crate::errors::{Result, ToolError};

/// The raw `git.hosts` YAML shape: hostname to provider-name pairs, in the order
/// the operator wrote them, keeping every duplicate rather than collapsing it.
///
/// `serde`'s built-in `HashMap<String, String>` deserialization silently keeps
/// only the last value for a repeated YAML key, which would make FR-NEW-003
/// (reject a duplicate host) undetectable: the duplicate would already be gone
/// by the time `validate_hosts` ran. The hand-written [`Deserialize`] below
/// collects every `(key, value)` pair `serde_yaml` hands it instead of folding
/// them into a map, so a duplicate survives to be rejected explicitly.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HostMap(pub Vec<(String, String)>);

impl<'de> Deserialize<'de> for HostMap {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct HostMapVisitor;
        impl<'de> serde::de::Visitor<'de> for HostMapVisitor {
            type Value = HostMap;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a mapping of hostname to provider")
            }
            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, String>()? {
                    entries.push(entry);
                }
                Ok(HostMap(entries))
            }
        }
        deserializer.deserialize_map(HostMapVisitor)
    }
}

/// Credential policy attached to a trusted git host.
///
/// `Github` also covers GitHub Enterprise Server: the credential shape and
/// `git.auth`'s validation are provider specific, not host specific (DEC-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Github,
    Gitlab,
    Generic,
    Anonymous,
}

impl Provider {
    /// The four accepted `git.hosts` values, lowercase exact. Listed here once so
    /// every "unknown provider" error names the same set the parser accepts.
    pub const ALL: [&'static str; 4] = ["github", "gitlab", "generic", "anonymous"];

    /// Lowercase-exact parse: `"GitHub"` and `""` are both rejected (E2E-NEW-004,
    /// E2E-NEW-005), because a provider value is either one of the four accepted
    /// strings or the map is misconfigured.
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "github" => Some(Self::Github),
            "gitlab" => Some(Self::Gitlab),
            "generic" => Some(Self::Generic),
            "anonymous" => Some(Self::Anonymous),
            _ => None,
        }
    }
}

/// One validated `git.hosts` entry: a bare hostname paired with its credential
/// policy, immutable once boot validation has published it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEntry {
    pub host: String,
    pub provider: Provider,
}

/// The published map, keyed by the exact hostname string the operator wrote.
///
/// `OnceLock` gives the container process lifetime; the `RwLock` inside lets
/// [`validate_hosts`] replace its contents exactly once in production (at boot)
/// while keeping the storage itself immutable-after-boot in spirit: nothing but
/// `validate_hosts` ever writes to it, and no tool or route calls that function
/// (FR-NEW-005).
static HOSTS: OnceLock<RwLock<HashMap<String, Provider>>> = OnceLock::new();

fn store() -> &'static RwLock<HashMap<String, Provider>> {
    HOSTS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Reject anything that is not a bare hostname: a scheme, a path, a port, or a
/// wildcard character (FR-NEW-004). `url::Host::parse` is the DRIFT-003
/// dependency: it is what actually rejects a scheme (`"https://github.com"`
/// fails IDNA validation on the embedded `/`), a path (`"github.com/org"`) and a
/// port (`"github.com:443"`), because none of those is a syntactically valid
/// host component. It does not reject a wildcard, so that check is explicit.
fn validate_host_key(host: &str) -> Result<()> {
    if host.contains('*') || host.contains('?') {
        return Err(ToolError::invalid_argument(format!(
            "git.hosts key '{host}' contains a wildcard character: host matching is exact only, \
             one key per hostname"
        )));
    }
    url::Host::parse(host).map_err(|e| {
        ToolError::invalid_argument(format!(
            "git.hosts key '{host}' is not a bare hostname (no scheme, path, or port \
             allowed): {e}"
        ))
    })?;
    Ok(())
}

/// Validate every `git.hosts` entry and, only once every entry is valid, publish
/// the map for [`resolve_host`]. A partial map is never published: an operator
/// never gets a boot that silently accepted half of what they wrote.
pub fn validate_hosts(cfg: &GitConfig) -> Result<()> {
    let mut resolved: HashMap<String, Provider> = HashMap::new();
    for (host, provider_raw) in &cfg.hosts.0 {
        validate_host_key(host)?;
        let provider = Provider::parse(provider_raw).ok_or_else(|| {
            ToolError::invalid_argument(format!(
                "git.hosts entry '{host}' has unknown provider '{provider_raw}', expected one \
                 of {}",
                Provider::ALL.join(", ")
            ))
        })?;
        if resolved.contains_key(host) {
            return Err(ToolError::invalid_argument(format!(
                "git.hosts contains duplicate host '{host}'"
            )));
        }
        resolved.insert(host.clone(), provider);
    }
    *store().write().expect("git host map lock poisoned") = resolved;
    Ok(())
}

/// Resolve a bare hostname to its declared credential policy. `O(1)`: a single
/// hash map lookup against the map [`validate_hosts`] published, never a scan.
///
/// A host absent from the map is `Err`, not `Provider::Anonymous`: exact
/// matching means an undeclared host is undeclared, not implicitly trusted.
pub fn resolve_host(host: &str) -> Result<Provider> {
    let map = store().read().expect("git host map lock poisoned");
    map.get(host)
        .copied()
        .ok_or_else(|| ToolError::not_found(format!("host '{host}' is not declared in git.hosts")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::GitConfig;

    /// Serializes every test that publishes to or reads from the process-global
    /// [`HOSTS`] map. `validate_hosts` replaces the whole map on every call, and
    /// `cargo test` runs unit tests in parallel threads within one process, so
    /// two tests touching the map concurrently would race on each other's
    /// contents. Tests that only assert a boot failure never reach the publish
    /// step, so they need no lock: nothing was ever written.
    pub(crate) fn lock_for_test() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn cfg_with_hosts(pairs: &[(&str, &str)]) -> GitConfig {
        let mut c = GitConfig::default();
        c.hosts.0 = pairs.iter().map(|(h, p)| (h.to_string(), p.to_string())).collect();
        c
    }

    const REFERENCE_MAP: &[(&str, &str)] = &[
        ("github.com", "github"),
        ("github.ibm.com", "github"),
        ("gitlab.acme.corp", "gitlab"),
        ("git.acme.internal", "generic"),
        ("public.example.org", "anonymous"),
    ];

    /// E2E-NEW-001: a valid map boots and every host resolves to its declared
    /// provider class.
    #[test]
    fn e2e_new_001_valid_map_boots_and_resolves_every_provider_class() {
        let _guard = lock_for_test();
        let cfg = cfg_with_hosts(REFERENCE_MAP);
        validate_hosts(&cfg).expect("the reference map is valid");

        assert_eq!(resolve_host("github.com").unwrap(), Provider::Github);
        assert_eq!(resolve_host("github.ibm.com").unwrap(), Provider::Github);
        assert_eq!(resolve_host("gitlab.acme.corp").unwrap(), Provider::Gitlab);
        assert_eq!(resolve_host("git.acme.internal").unwrap(), Provider::Generic);
        assert_eq!(resolve_host("public.example.org").unwrap(), Provider::Anonymous);
    }

    /// E2E-NEW-002: two hosts may share one provider and resolve independently.
    #[test]
    fn e2e_new_002_two_hosts_may_share_one_provider() {
        let _guard = lock_for_test();
        let cfg = cfg_with_hosts(&[("github.com", "github"), ("github.ibm.com", "github")]);
        validate_hosts(&cfg).expect("two hosts on the same provider is valid");

        assert_eq!(resolve_host("github.com").unwrap(), Provider::Github);
        assert_eq!(resolve_host("github.ibm.com").unwrap(), Provider::Github);
    }

    /// E2E-NEW-003: an unknown provider value fails boot, naming the host and
    /// listing the accepted values.
    #[test]
    fn e2e_new_003_unknown_provider_value_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.ibm.com", "githib")]);
        let e = validate_hosts(&cfg).expect_err("githib is not an accepted provider");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("github.ibm.com"), "{}", e.message);
        for name in Provider::ALL {
            assert!(e.message.contains(name), "{name} should be listed: {}", e.message);
        }
    }

    /// E2E-NEW-004: provider values are lowercase-exact, so `GitHub` fails boot.
    #[test]
    fn e2e_new_004_uppercase_provider_value_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com", "GitHub")]);
        let e = validate_hosts(&cfg).expect_err("provider values are lowercase-exact");
        assert!(e.message.contains("github.com"), "{}", e.message);
    }

    /// E2E-NEW-005: an empty provider value fails boot, naming the host.
    #[test]
    fn e2e_new_005_empty_provider_value_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com", "")]);
        let e = validate_hosts(&cfg).expect_err("an empty provider value is invalid");
        assert!(e.message.contains("github.com"), "{}", e.message);
    }

    /// E2E-NEW-006: a duplicate host fails boot naming the host, and neither
    /// entry is silently chosen (nothing is published: the loop errors before
    /// `store()` is ever written for this map).
    #[test]
    fn e2e_new_006_duplicate_host_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com", "github"), ("github.com", "anonymous")]);
        let e = validate_hosts(&cfg).expect_err("github.com appears twice");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("github.com"), "{}", e.message);
        assert!(e.message.contains("duplicate"), "{}", e.message);
    }

    /// E2E-NEW-007: a host with a scheme fails boot, naming the offending key.
    #[test]
    fn e2e_new_007_host_with_a_scheme_fails_boot() {
        let cfg = cfg_with_hosts(&[("https://github.com", "github")]);
        let e = validate_hosts(&cfg).expect_err("a scheme is not a bare hostname");
        assert!(e.message.contains("https://github.com"), "{}", e.message);
    }

    /// E2E-NEW-008: a host with a path fails boot, naming the offending key.
    #[test]
    fn e2e_new_008_host_with_a_path_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com/org", "github")]);
        let e = validate_hosts(&cfg).expect_err("a path is not a bare hostname");
        assert!(e.message.contains("github.com/org"), "{}", e.message);
    }

    /// E2E-NEW-009: a host with a port fails boot, naming the key.
    #[test]
    fn e2e_new_009_host_with_a_port_fails_boot() {
        let cfg = cfg_with_hosts(&[("github.com:443", "github")]);
        let e = validate_hosts(&cfg).expect_err("a port is not a bare hostname");
        assert!(e.message.contains("github.com:443"), "{}", e.message);
    }

    /// E2E-NEW-010: a wildcard host fails boot, naming the key, and the message
    /// does not suggest wildcards are supported (DEC-015).
    #[test]
    fn e2e_new_010_wildcard_host_fails_boot() {
        let cfg = cfg_with_hosts(&[("*.acme.corp", "gitlab")]);
        let e = validate_hosts(&cfg).expect_err("wildcards are not supported");
        assert!(e.message.contains("*.acme.corp"), "{}", e.message);
        assert!(
            !e.message.to_ascii_lowercase().contains("supported"),
            "the message must not suggest wildcards are supported: {}",
            e.message
        );
    }

    /// E2E-NEW-013: an empty map boots, and a host is then undeclared rather
    /// than implicitly anonymous.
    #[test]
    fn e2e_new_013_empty_map_boots() {
        let _guard = lock_for_test();
        let cfg = cfg_with_hosts(&[]);
        validate_hosts(&cfg).expect("an empty git.hosts map is valid");

        let e = resolve_host("github.com").expect_err("github.com was never declared");
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);
        assert!(e.message.contains("github.com"), "{}", e.message);
    }

    /// E2E-NEW-014: an absent `hosts` key behaves exactly like an empty map,
    /// because `GitConfig` derives `Default` for it.
    #[test]
    fn e2e_new_014_absent_map_section_boots() {
        let _guard = lock_for_test();
        let cfg = GitConfig::default();
        assert!(cfg.hosts.0.is_empty(), "an absent hosts key must default to empty");
        validate_hosts(&cfg).expect("a config with no hosts key is valid");

        let e = resolve_host("github.com").expect_err("no host was ever declared");
        assert_eq!(e.code, crate::errors::code::NOT_FOUND);
    }

    /// E2E-NEW-017: no tool in the registry accepts input that changes a
    /// host-to-provider entry. `git.hosts` is config-only surface: this
    /// enumerates the real, currently registered tool set (the families this
    /// story touches nothing in) and asserts none of them exposes a `hosts`
    /// property.
    #[test]
    fn e2e_new_017_no_registered_tool_can_mutate_the_host_map() {
        let mut reg = crate::mcp::ToolRegistry::new();
        crate::tools::register_fs(&mut reg);
        crate::tools::admin::register(&mut reg);
        crate::tools::git::register(&mut reg);
        crate::tools::git_auth::register(&mut reg);

        for &name in reg.names() {
            let tool = reg.resolve(name).expect("just listed");
            let schema = tool.schema.input_schema();
            assert!(
                schema.get("properties").and_then(|p| p.get("hosts")).is_none(),
                "tool '{name}' must not accept a 'hosts' property"
            );
            assert!(
                !name.to_ascii_lowercase().contains("host"),
                "tool '{name}' name must not reference hosts"
            );
        }
    }

    /// E2E-NEW-111: no request a web screen could issue changes a
    /// host-to-provider entry. No such screen exists in this crate yet (it is
    /// out of this story's scope), and git has no REST surface at all
    /// (`api/dataplane.rs` carries none), so this is verified by scanning the
    /// one REST surface that exists for any reference to `git.hosts`.
    #[test]
    fn e2e_new_111_the_screen_cannot_modify_the_host_map() {
        let dataplane = include_str!("../api/dataplane.rs");
        assert!(
            !dataplane.contains("git.hosts") && !dataplane.contains("git::remote"),
            "api/dataplane.rs must not read or write git.hosts: git has no REST surface"
        );
    }

    /// E2E-NEW-224: validation and resolution live in one module. This proves
    /// existence and reachability by calling both functions through their full
    /// path, and proves `config.rs` calls `validate_hosts` exactly once and
    /// inspects no entry itself by scanning its own source.
    #[test]
    fn e2e_new_224_validation_and_resolution_live_in_one_module() {
        let _guard = lock_for_test();
        let cfg = GitConfig::default();
        crate::git::remote::validate_hosts(&cfg).expect("an empty map validates");
        let _ = crate::git::remote::resolve_host("git.unknown.test");

        let config_src = include_str!("../config.rs");
        assert!(config_src.contains("pub hosts:"), "GitConfig must declare the hosts field");
        assert_eq!(
            config_src.matches("validate_hosts(").count(),
            1,
            "config.rs must call validate_hosts exactly once"
        );
        for needle in [".hosts.0.iter()", ".hosts.0.contains", "Provider::parse"] {
            assert!(
                !config_src.contains(needle),
                "config.rs must not inspect a git.hosts entry itself (found {needle:?})"
            );
        }
    }

    /// E2E-NEW-225: boot validation still fires from `ServerConfig::validate`.
    #[test]
    fn e2e_new_225_boot_validation_fires_from_server_config_validate() {
        let mut cfg = crate::config::ServerConfig::default();
        cfg.git.hosts.0 = vec![("github.ibm.com".to_string(), "githib".to_string())];
        let e = cfg.validate().expect_err("an unknown provider must fail ServerConfig::validate");
        assert_eq!(e.code, crate::errors::code::INVALID_ARGUMENT);
        assert!(e.message.contains("github.ibm.com"), "{}", e.message);
    }

    /// E2E-NEW-226: a valid map passes validation from the same path, and every
    /// host resolves through `git::remote::resolve_host` afterwards.
    #[test]
    fn e2e_new_226_a_valid_map_passes_validation_from_the_same_path() {
        let _guard = lock_for_test();
        let mut cfg = crate::config::ServerConfig::default();
        cfg.git.hosts.0 =
            REFERENCE_MAP.iter().map(|(h, p)| (h.to_string(), p.to_string())).collect();
        cfg.validate().expect("the reference map is valid");

        for (host, provider) in [
            ("github.com", Provider::Github),
            ("github.ibm.com", Provider::Github),
            ("gitlab.acme.corp", Provider::Gitlab),
            ("git.acme.internal", Provider::Generic),
            ("public.example.org", Provider::Anonymous),
        ] {
            assert_eq!(crate::git::remote::resolve_host(host).unwrap(), provider);
        }
    }
}
