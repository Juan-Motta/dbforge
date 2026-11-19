//! How each agent is pointed at DBForge's MCP server, as the former web UI's
//! `setup.ts`: the steps to add it, a prompt that has the agent add it
//! itself, and the note for a project's instructions file that has the
//! agent use it. Every format here was checked against the agent's own
//! documentation. Pure functions, so they are tested without a window.

use serde_json::json;

/// The environment variable agents that keep no secrets in their config read the token from.
pub const TOKEN_VARIABLE: &str = "DBFORGE_MCP_TOKEN";

/// The argument that starts DBForge as an agent's MCP server over stdio, not as the app.
pub const STDIO_FLAG: &str = dbforge_core::MCP_STDIO_FLAG;

/// The agents the settings write steps for, in the order they offer them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentId {
    Claude,
    Codex,
    Opencode,
    Pi,
    Other,
}

pub const AGENTS: [AgentId; 5] = [AgentId::Claude, AgentId::Codex, AgentId::Opencode, AgentId::Pi, AgentId::Other];

/// How an agent reaches DBForge: it starts DBForge's executable itself and
/// talks to it over stdio, which stays connected while the app is closed or
/// restarts; or it connects to the open app over HTTP with the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    Http,
}

pub const TRANSPORTS: [Transport; 2] = [Transport::Stdio, Transport::Http];

/// Where an agent reaches DBForge, with what it needs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint<'a> {
    Stdio { executable: &'a str },
    Http { url: &'a str, token: &'a str },
}

impl Endpoint<'_> {
    pub fn transport(&self) -> Transport {
        match self {
            Self::Stdio { .. } => Transport::Stdio,
            Self::Http { .. } => Transport::Http,
        }
    }
}

/// What a step asks of the user, so the screen can say it in their language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Terminal,
    Profile,
    Config,
    Connect,
    Launch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: StepKind,
    /// The file a `Config` step goes into.
    pub file: Option<&'static str>,
    pub text: String,
}

impl Step {
    fn new(kind: StepKind, text: String) -> Self {
        Self { kind, file: None, text }
    }

    fn config(file: &'static str, text: String) -> Self {
        Self { kind: StepKind::Config, file: Some(file), text }
    }
}

/// The server's address on `port`, as agents reach it.
pub fn server_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

/// A port the server can take: 1024 to 65535. Read as JavaScript's
/// `Number` reads the field (spaces around, `+`, a decimal point or an
/// exponent, hexadecimal), then kept only when it is a whole number in range.
pub fn parse_port(text: &str) -> Option<u16> {
    let text = text.trim();
    let radix = |prefix: &str, radix: u32| {
        let digits = text.get(..2).filter(|start| start.eq_ignore_ascii_case(prefix)).map(|_| &text[2..])?;
        u64::from_str_radix(digits, radix).ok().map(|value| value as f64)
    };
    let number = radix("0x", 16).or_else(|| radix("0o", 8)).or_else(|| radix("0b", 2)).or_else(|| {
        // `Number` takes `Infinity` but no other word; Rust's `f64` takes `inf` and `nan` too.
        let numeric = text.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'));
        numeric.then(|| text.parse::<f64>().ok()).flatten()
    })?;
    (number.fract() == 0. && (1024. ..=65535.).contains(&number)).then_some(number as u16)
}

const OPENCODE_CONFIG: &str = "~/.config/opencode/opencode.json";

/// The query tool may take a minute; OpenCode's default wait is 30 seconds.
const OPENCODE_TIMEOUT: u32 = 70000;

/// `path` as one argument on its own system's shells: a Windows path in
/// double quotes (cmd and PowerShell; it can't contain one), any other in
/// single quotes. Left bare when it has nothing a shell would split or expand.
pub fn shell_argument(path: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./\\-".contains(c);
    if !path.is_empty() && path.chars().all(plain) {
        return path.to_owned();
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        return format!("\"{path}\"");
    }
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// JSON as the former web UI wrote it (`JSON.stringify(value, null, 2)`): keys in
/// the order written, two spaces of indent.
fn pretty(value: serde_json::Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// The steps that add DBForge to `agent`, reached through `endpoint`.
pub fn setup_steps(agent: AgentId, endpoint: Endpoint) -> Vec<Step> {
    match endpoint {
        Endpoint::Stdio { executable } => stdio_steps(agent, executable),
        Endpoint::Http { url, token } => http_steps(agent, url, token),
    }
}

/// Each agent's way of adding a server it starts itself, for every project.
fn stdio_steps(agent: AgentId, executable: &str) -> Vec<Step> {
    let command = format!("{} {STDIO_FLAG}", shell_argument(executable));
    match agent {
        // `--scope user`: the default scope ties the server to the folder the command runs in.
        AgentId::Claude => {
            vec![Step::new(StepKind::Terminal, format!("claude mcp add --scope user dbforge -- {command}"))]
        }
        AgentId::Codex => vec![Step::new(StepKind::Terminal, format!("codex mcp add dbforge -- {command}"))],
        AgentId::Opencode => {
            let config = json!({
                "$schema": "https://opencode.ai/config.json",
                "mcp": {
                    "dbforge": {
                        "type": "local",
                        "command": [executable, STDIO_FLAG],
                        "enabled": true,
                        "timeout": OPENCODE_TIMEOUT,
                    },
                },
            });
            vec![Step::config(OPENCODE_CONFIG, pretty(config))]
        }
        AgentId::Pi => vec![Step::new(StepKind::Terminal, format!("pi mcp add dbforge -- {command}"))],
        AgentId::Other => {
            let config = json!({ "mcpServers": { "dbforge": { "command": executable, "args": [STDIO_FLAG] } } });
            vec![
                Step::new(
                    StepKind::Launch,
                    format!("Command: {executable}\nArguments: {STDIO_FLAG}\nTransport: stdio"),
                ),
                Step::config("mcp.json", pretty(config)),
            ]
        }
    }
}

/// Each agent's way of adding a server it reaches over HTTP, with `token` written in.
fn http_steps(agent: AgentId, url: &str, token: &str) -> Vec<Step> {
    let bearer = format!("Bearer {token}");
    match agent {
        // `--scope user`: the default scope ties the server to the folder the command runs in.
        AgentId::Claude => vec![Step::new(
            StepKind::Terminal,
            format!("claude mcp add --transport http --scope user dbforge {url} --header \"Authorization: {bearer}\""),
        )],
        AgentId::Codex => vec![
            Step::new(
                StepKind::Terminal,
                format!("codex mcp add dbforge --url {url} --bearer-token-env-var {TOKEN_VARIABLE}"),
            ),
            Step::new(StepKind::Profile, format!("export {TOKEN_VARIABLE}={token}")),
        ],
        AgentId::Opencode => {
            let config = json!({
                "$schema": "https://opencode.ai/config.json",
                "mcp": {
                    "dbforge": {
                        "type": "remote",
                        "url": url,
                        "enabled": true,
                        "oauth": false,
                        "timeout": OPENCODE_TIMEOUT,
                        "headers": { "Authorization": bearer },
                    },
                },
            });
            vec![Step::config(OPENCODE_CONFIG, pretty(config))]
        }
        AgentId::Pi => vec![Step::new(
            StepKind::Terminal,
            format!("pi mcp add dbforge --url {url} --header \"Authorization={bearer}\""),
        )],
        AgentId::Other => {
            let config = json!({ "mcpServers": { "dbforge": { "url": url, "headers": { "Authorization": bearer } } } });
            vec![
                Step::new(
                    StepKind::Connect,
                    format!("URL: {url}\nTransport: Streamable HTTP\nHeader: Authorization: {bearer}"),
                ),
                Step::config("mcp.json", pretty(config)),
            ]
        }
    }
}

/// Where `agent` reads a project's standing instructions.
pub fn instructions_file(agent: AgentId) -> &'static str {
    if agent == AgentId::Claude { "CLAUDE.md" } else { "AGENTS.md" }
}

/// How `agent` checks that the server is connected, when it has a command for it.
fn check_command(agent: AgentId) -> Option<&'static str> {
    match agent {
        AgentId::Claude => Some("claude mcp get dbforge"),
        AgentId::Codex => Some("codex mcp list"),
        AgentId::Opencode => Some("opencode mcp list"),
        AgentId::Pi => Some("pi mcp list"),
        AgentId::Other => None,
    }
}

/// The note for a project's instructions file that has the agent use DBForge for its databases.
pub fn project_note() -> String {
    [
        "## Databases",
        "",
        "Query and change this project’s databases through the `dbforge` MCP server, not with",
        "database clients or credentials: DBForge holds the connections and their passwords.",
        "Start with `list_connections`; each connection says its language and whether you may",
        "change data. DBForge must be open for the server to answer.",
    ]
    .join("\n")
}

/// A prompt that has `agent` add DBForge itself, then say so in the project's instructions.
pub fn agent_prompt(agent: AgentId, endpoint: Endpoint) -> String {
    let http = endpoint.transport() == Transport::Http;
    let must_be_open = if http { " (DBForge must be open)" } else { "" };
    let how: Vec<String> = setup_steps(agent, endpoint)
        .into_iter()
        .map(|step| match step.kind {
            StepKind::Terminal => format!("Run:\n\n{}", step.text),
            StepKind::Profile => {
                format!("Add this line to my shell profile (~/.zshrc or ~/.bashrc) and load it:\n\n{}", step.text)
            }
            StepKind::Config => format!(
                "Merge this into {}, keeping what is already there:\n\n{}",
                step.file.unwrap_or_default(),
                step.text
            ),
            StepKind::Connect => format!("Its connection details:\n\n{}", step.text),
            StepKind::Launch => format!("How to start it (over stdio):\n\n{}", step.text),
        })
        .collect();
    let mut parts = vec![
        if agent == AgentId::Other {
            "Add the MCP server \"dbforge\" (the DBForge database app on this computer) to your MCP configuration, \
             available in every project."
        } else {
            "Add the MCP server \"dbforge\" (the DBForge database app on this computer) to your configuration, \
             available in every project."
        }
        .to_owned(),
        how.join("\n\n"),
        // Over stdio the agent starts DBForge's side itself; over HTTP the app must be open to answer.
        match check_command(agent) {
            Some(check) => format!("Then check it is connected with `{check}`{must_be_open}, and tell me if it isn't."),
            None => format!("Then check it is connected{must_be_open}, and tell me if it isn’t."),
        },
        format!(
            "Finally, add this section to the {} of the current project, creating the file if needed:\n\n{}",
            instructions_file(agent),
            project_note()
        ),
    ];
    if http {
        parts.push("The token is a secret: don’t print it back, and don’t put it in the project.".to_owned());
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    const URL: &str = "http://127.0.0.1:7419/mcp";
    const HTTP: Endpoint = Endpoint::Http { url: URL, token: "abc" };
    const MAC: Endpoint = Endpoint::Stdio { executable: "/Applications/DBForge.app/Contents/MacOS/DBForge" };

    fn texts(agent: AgentId, endpoint: Endpoint) -> Vec<String> {
        setup_steps(agent, endpoint).into_iter().map(|step| step.text).collect()
    }

    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).expect("JSON")
    }

    #[test]
    fn serves_on_the_loopback_address_and_takes_only_unprivileged_ports() {
        assert_eq!(server_url(7419), URL);
        assert_eq!(parse_port(" 8080 "), Some(8080));
        assert_eq!(parse_port("80"), None);
        assert_eq!(parse_port("70000"), None);
        assert_eq!(parse_port("80a"), None);
        // As JavaScript's `Number` reads them.
        assert_eq!(parse_port("8080.0"), Some(8080));
        assert_eq!(parse_port("8080.5"), None);
        assert_eq!(parse_port("0x1F90"), Some(8080));
        assert_eq!(parse_port(""), None);
        assert_eq!(parse_port("inf"), None);
        assert_eq!(parse_port("1024"), Some(1024));
        assert_eq!(parse_port("65535"), Some(65535));
    }

    #[test]
    fn writes_each_agent_s_own_way_of_adding_a_server_for_every_project() {
        assert_eq!(
            texts(AgentId::Claude, HTTP)[0],
            "claude mcp add --transport http --scope user dbforge http://127.0.0.1:7419/mcp --header \
             \"Authorization: Bearer abc\""
        );
        assert_eq!(
            texts(AgentId::Codex, HTTP),
            [
                "codex mcp add dbforge --url http://127.0.0.1:7419/mcp --bearer-token-env-var DBFORGE_MCP_TOKEN",
                "export DBFORGE_MCP_TOKEN=abc",
            ]
        );
        assert_eq!(
            texts(AgentId::Pi, HTTP)[0],
            "pi mcp add dbforge --url http://127.0.0.1:7419/mcp --header \"Authorization=Bearer abc\""
        );
        let opencode = &setup_steps(AgentId::Opencode, HTTP)[0];
        assert_eq!(opencode.file, Some("~/.config/opencode/opencode.json"));
        let server = &parsed(&opencode.text)["mcp"]["dbforge"];
        assert_eq!(server["type"], "remote");
        assert_eq!(server["url"], URL);
        assert_eq!(server["oauth"], false);
        assert_eq!(server["headers"], json!({ "Authorization": "Bearer abc" }));
        let other = setup_steps(AgentId::Other, HTTP);
        assert_eq!(
            parsed(&other[1].text),
            json!({ "mcpServers": { "dbforge": { "url": URL, "headers": { "Authorization": "Bearer abc" } } } })
        );
    }

    #[test]
    fn writes_json_as_the_web_ui_did() {
        let other = setup_steps(AgentId::Other, MAC);
        assert_eq!(
            other[1].text,
            "{\n  \"mcpServers\": {\n    \"dbforge\": {\n      \"command\": \
             \"/Applications/DBForge.app/Contents/MacOS/DBForge\",\n      \"args\": [\n        \"--mcp-stdio\"\n      \
             ]\n    }\n  }\n}"
        );
        // Keys in the order written: `$schema` first, as `JSON.stringify` kept them.
        assert!(setup_steps(AgentId::Opencode, MAC)[0].text.starts_with("{\n  \"$schema\""));
    }

    #[test]
    fn prompts_the_agent_to_add_the_server_check_it_and_note_it_in_the_project_s_instructions() {
        let prompt = agent_prompt(AgentId::Codex, HTTP);
        assert!(prompt.contains(&texts(AgentId::Codex, HTTP)[0]));
        assert!(prompt.contains("export DBFORGE_MCP_TOKEN=abc"));
        assert!(prompt.contains("codex mcp list"));
        assert!(prompt.contains("AGENTS.md"));
        assert!(prompt.contains(&project_note()));
        assert_eq!(instructions_file(AgentId::Claude), "CLAUDE.md");
        assert!(!agent_prompt(AgentId::Other, HTTP).contains("mcp list"));
        // Over HTTP the wording stays as it was before stdio existed.
        assert!(agent_prompt(AgentId::Claude, HTTP).contains(
            "Then check it is connected with `claude mcp get dbforge` (DBForge must be open), and tell me if it isn't."
        ));
        assert!(
            agent_prompt(AgentId::Other, HTTP)
                .contains("Then check it is connected (DBForge must be open), and tell me if it isn’t.")
        );
        assert!(agent_prompt(AgentId::Claude, HTTP).ends_with("don’t put it in the project."));
    }

    #[test]
    fn has_each_agent_start_dbforge_itself_with_no_token_on_every_platform() {
        assert_eq!(
            texts(AgentId::Claude, MAC),
            ["claude mcp add --scope user dbforge -- /Applications/DBForge.app/Contents/MacOS/DBForge --mcp-stdio"]
        );
        assert_eq!(
            texts(AgentId::Codex, MAC),
            ["codex mcp add dbforge -- /Applications/DBForge.app/Contents/MacOS/DBForge --mcp-stdio"]
        );
        assert_eq!(
            texts(AgentId::Pi, MAC)[0],
            "pi mcp add dbforge -- /Applications/DBForge.app/Contents/MacOS/DBForge --mcp-stdio"
        );
        let server = &parsed(&setup_steps(AgentId::Opencode, MAC)[0].text)["mcp"]["dbforge"];
        assert_eq!(server["type"], "local");
        assert_eq!(server["command"], json!(["/Applications/DBForge.app/Contents/MacOS/DBForge", "--mcp-stdio"]));
        assert_eq!(server["enabled"], true);
        let other = setup_steps(AgentId::Other, MAC);
        assert_eq!(other[0].kind, StepKind::Launch);
        assert_eq!(
            parsed(&other[1].text),
            json!({ "mcpServers": { "dbforge": {
                "command": "/Applications/DBForge.app/Contents/MacOS/DBForge",
                "args": ["--mcp-stdio"],
            } } })
        );
        let prompt = agent_prompt(AgentId::Claude, MAC);
        assert!(prompt.contains(&texts(AgentId::Claude, MAC)[0]));
        assert!(!prompt.contains("token"));
    }

    #[test]
    fn quotes_an_executable_path_the_way_its_own_system_s_shells_read_it() {
        assert_eq!(shell_argument("/usr/bin/dbforge"), "/usr/bin/dbforge");
        assert_eq!(
            shell_argument("C:\\Program Files\\DBForge\\DBForge.exe"),
            "\"C:\\Program Files\\DBForge\\DBForge.exe\""
        );
        assert_eq!(
            shell_argument("/home/ana/My Apps/DBForge's.AppImage"),
            "'/home/ana/My Apps/DBForge'\\''s.AppImage'"
        );
        let windows = Endpoint::Stdio { executable: "C:\\Users\\Ana\\AppData\\Local\\DBForge\\DBForge.exe" };
        assert_eq!(
            texts(AgentId::Claude, windows)[0],
            "claude mcp add --scope user dbforge -- C:\\Users\\Ana\\AppData\\Local\\DBForge\\DBForge.exe --mcp-stdio"
        );
    }
}
