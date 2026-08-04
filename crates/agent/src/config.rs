//! Agent configuration, loaded from YAML.
//!
//! The key names and defaults match the reference `config/agent_test.yaml` exactly, so
//! the same file drives either implementation.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// How to reach the MCP server and how to authenticate.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub url: String,
    /// Fallback token used when `--user` is not given.
    pub token: String,
    /// Header carrying the bearer token. The server reads the forwarded header first,
    /// which is what a gateway in front of it would set.
    pub auth_header: String,
    /// Directory holding one raw JWT per user, file name equal to the `--user` value.
    pub tokens_dir: String,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:5002/mcp".to_string(),
            token: String::new(),
            auth_header: "X-Forwarded-Authorization".to_string(),
            tokens_dir: ".agent_keys".to_string(),
        }
    }
}

/// The chat endpoint. Any OpenAI compatible `/chat/completions` will do.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    /// Environment variable holding the key. Checked before `api_key` so a secret never
    /// has to sit in the YAML.
    pub api_key_env: String,
    /// Ceiling on one LLM turn, so a hung endpoint cannot freeze the agent forever.
    pub timeout_seconds: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: String::new(),
            api_key_env: "AGENT_TEST_API_KEY".to_string(),
            timeout_seconds: 180,
        }
    }
}

/// The whole file.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    pub mcp: McpConfig,
    pub llm: LlmConfig,
    pub system_prompt: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            mcp: McpConfig::default(),
            llm: LlmConfig::default(),
            system_prompt: "You are a helpful assistant with access to a filesystem via \
                 MCP tools (fs.* and admin.*). Use the tools to answer the user's \
                 questions about their files."
                .to_string(),
        }
    }
}

impl AgentConfig {
    /// Read and parse the file. Unknown keys are ignored so a newer config still loads.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("config file not found: {}", path.display()))?;
        serde_yaml::from_str(&text)
            .with_context(|| format!("invalid YAML in {}", path.display()))
    }

    /// The API key, preferring the environment so the YAML can stay committable.
    pub fn api_key(&self) -> Option<String> {
        if !self.llm.api_key_env.is_empty()
            && let Ok(v) = std::env::var(&self.llm.api_key_env)
            && !v.trim().is_empty()
        {
            return Some(v.trim().to_string());
        }
        let k = self.llm.api_key.trim();
        (!k.is_empty()).then(|| k.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_yaml(body: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f
    }

    #[test]
    fn defaults_apply_to_every_missing_key() {
        let f = write_yaml("mcp:\n  url: http://x/mcp\n");
        let c = AgentConfig::load(f.path()).unwrap();
        assert_eq!(c.mcp.url, "http://x/mcp");
        assert_eq!(c.mcp.auth_header, "X-Forwarded-Authorization", "default kept");
        assert_eq!(c.mcp.tokens_dir, ".agent_keys");
        assert_eq!(c.llm.timeout_seconds, 180);
        assert!(c.system_prompt.contains("filesystem"));
    }

    #[test]
    fn an_unknown_key_does_not_break_loading() {
        let f = write_yaml("mcp:\n  url: http://x/mcp\n  future_option: 7\ntop_level: nope\n");
        let c = AgentConfig::load(f.path()).unwrap();
        assert_eq!(c.mcp.url, "http://x/mcp");
    }

    #[test]
    fn the_full_reference_shape_parses() {
        let f = write_yaml(
            "mcp:\n  url: http://127.0.0.1:5002/mcp\n  token: \"\"\n  \
             tokens_dir: .agent_keys\n  auth_header: X-Forwarded-Authorization\n\
             llm:\n  base_url: https://api.example.com/v1\n  model: claude-haiku-4-5\n  \
             api_key: \"\"\n  api_key_env: SOME_KEY\n  timeout_seconds: 42\n\
             system_prompt: >\n  Be helpful.\n",
        );
        let c = AgentConfig::load(f.path()).unwrap();
        assert_eq!(c.llm.model, "claude-haiku-4-5");
        assert_eq!(c.llm.timeout_seconds, 42);
        assert_eq!(c.llm.api_key_env, "SOME_KEY");
        assert_eq!(c.system_prompt.trim(), "Be helpful.");
    }

    #[test]
    fn a_missing_file_is_an_error_naming_the_path() {
        let e = AgentConfig::load(Path::new("/nope/agent.yaml")).unwrap_err();
        assert!(e.to_string().contains("/nope/agent.yaml"), "got {e}");
    }

    #[test]
    fn the_environment_wins_over_the_inline_key() {
        // A unique name per test process, so a parallel test cannot observe it.
        let var = "AGENT_CFG_TEST_KEY_A";
        let mut c = AgentConfig::default();
        c.llm.api_key = "from-yaml".into();
        c.llm.api_key_env = var.into();
        assert_eq!(c.api_key().as_deref(), Some("from-yaml"), "unset var falls back");

        // SAFETY: single threaded within this test, and the name is unique to it.
        unsafe { std::env::set_var(var, "  from-env  ") };
        assert_eq!(c.api_key().as_deref(), Some("from-env"), "trimmed and preferred");
        unsafe { std::env::remove_var(var) };
    }

    #[test]
    fn a_blank_key_reads_as_absent() {
        let mut c = AgentConfig::default();
        c.llm.api_key = "   ".into();
        c.llm.api_key_env = "AGENT_CFG_TEST_KEY_B".into();
        assert!(c.api_key().is_none());
    }
}
