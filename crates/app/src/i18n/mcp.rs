//! The texts of Settings › Agents (MCP) and of the dialog asking the user to
//! approve an agent's change, as the web UI's `mcp/messages.ts`.

/// The MCP settings' and approvals' texts in one language.
pub struct McpMessages {
    // The approval dialog
    pub approval_title: &'static str,
    /// An agent that gave no name.
    pub an_agent: &'static str,
    /// The agent, then the connection's name.
    pub approval_who: fn(&str, &str) -> String,
    pub in_database: fn(&str) -> String,
    pub approval_note: &'static str,
    pub approve: &'static str,
    pub decline: &'static str,
    /// The dialog's close button, which declines.
    pub close: &'static str,
    // The settings
    pub enable: &'static str,
    pub enable_description: &'static str,
    pub listening: &'static str,
    pub failed: &'static str,
    pub port: &'static str,
    pub port_help: &'static str,
    pub port_invalid: &'static str,
    pub token: &'static str,
    pub show: &'static str,
    pub hide: &'static str,
    pub copy: &'static str,
    pub regenerate: &'static str,
    pub regenerate_confirm: &'static str,
    pub cancel: &'static str,
    pub add_to_agents: &'static str,
    pub claude_code: &'static str,
    pub codex: &'static str,
    pub opencode: &'static str,
    pub pi: &'static str,
    pub other_agent: &'static str,
    pub step_terminal: &'static str,
    pub step_profile: &'static str,
    /// The file a step's text goes into.
    pub step_config: fn(&str) -> String,
    pub step_connect: &'static str,
    pub step_launch: &'static str,
    pub transport: &'static str,
    pub transport_stdio: &'static str,
    pub transport_http: &'static str,
    pub transport_stdio_note: &'static str,
    pub transport_http_note: &'static str,
    pub prompt_title: &'static str,
    pub prompt_help: &'static str,
    pub prompt_help_stdio: &'static str,
    /// The agent's instructions file (`CLAUDE.md`, `AGENTS.md`).
    pub note_title: fn(&str) -> String,
    pub access_note: &'static str,
}
