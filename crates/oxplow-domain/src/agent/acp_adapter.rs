//! An ACP agent's program, as data: what a declaration's `config:` says
//! (the `oxplow:acp-adapter` built-in), the way a collector is data.

use std::collections::BTreeMap;

use serde::Deserialize;

/// How an ACP agent gets oxplow's system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPromptVia {
    /// `_meta.systemPrompt.append` on `session/new` (the Claude adapter).
    Meta,
    /// A block ahead of the first prompt of a new session.
    Prompt,
}

/// One ACP agent's program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpAdapter {
    /// What a session's `acp_agent` names (`gemini`).
    pub id: String,
    pub title: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub system_prompt: SystemPromptVia,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    system_prompt: Option<SystemPromptVia>,
}

impl AcpAdapter {
    /// The adapter a declaration `id` configures with `config`
    /// (`{ command, args?, env?, systemPrompt?: meta|prompt }`).
    pub fn from_config(id: &str, title: &str, config: &serde_json::Value) -> Result<Self, String> {
        let c: Config =
            serde_json::from_value(config.clone()).map_err(|e| format!("config: {e}"))?;
        Ok(Self {
            id: id.into(),
            title: title.into(),
            command: c.command,
            args: c.args,
            env: c.env,
            system_prompt: c.system_prompt.unwrap_or(SystemPromptVia::Prompt),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_adapter_is_its_declarations_config() {
        let a = AcpAdapter::from_config(
            "claude",
            "Claude",
            &serde_json::json!({ "command": "claude-agent-acp", "systemPrompt": "meta" }),
        )
        .unwrap();
        assert_eq!(
            (a.command.as_str(), a.args.len(), a.system_prompt),
            ("claude-agent-acp", 0, SystemPromptVia::Meta)
        );
        let g = AcpAdapter::from_config(
            "gemini",
            "Gemini",
            &serde_json::json!({ "command": "gemini", "args": ["--acp"] }),
        )
        .unwrap();
        assert_eq!(
            (g.args.clone(), g.system_prompt),
            (vec!["--acp".to_string()], SystemPromptVia::Prompt)
        );
        assert!(AcpAdapter::from_config("x", "x", &serde_json::json!({})).is_err());
    }
}
