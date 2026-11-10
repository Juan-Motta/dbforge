/**
 * How each agent is pointed at DBForge's MCP server: the steps to add it, a
 * prompt that has the agent add it itself, and the note for a project's
 * instructions file that has the agent use it. Every format here was checked
 * against the agent's own documentation.
 */

/** The environment variable agents that keep no secrets in their config read the token from. */
export const TOKEN_VARIABLE = 'DBFORGE_MCP_TOKEN'

export type AgentId = 'claude' | 'codex' | 'opencode' | 'pi' | 'other'

export const AGENTS: AgentId[] = ['claude', 'codex', 'opencode', 'pi', 'other']

/** What a step asks of the user, so the screen can say it in their language. */
export type StepKind = 'terminal' | 'profile' | 'config' | 'connect'

export interface Step {
  kind: StepKind
  /** The file a `config` step goes into. */
  file?: string
  text: string
}

/** The server's address on `port`, as agents reach it. */
export function serverUrl(port: number): string {
  return `http://127.0.0.1:${port}/mcp`
}

/** A port the server can take: 1024 to 65535. */
export function parsePort(text: string): number | null {
  const port = Number(text.trim())
  return Number.isInteger(port) && port >= 1024 && port <= 65535 ? port : null
}

const OPENCODE_CONFIG = '~/.config/opencode/opencode.json'

/** The steps that add DBForge to `agent`, with `token` written in. */
export function setupSteps(agent: AgentId, url: string, token: string): Step[] {
  const bearer = `Bearer ${token}`
  switch (agent) {
    case 'claude':
      // `--scope user`: the default scope ties the server to the folder the command runs in.
      return [
        {
          kind: 'terminal',
          text: `claude mcp add --transport http --scope user dbforge ${url} --header "Authorization: ${bearer}"`,
        },
      ]
    case 'codex':
      return [
        {
          kind: 'terminal',
          text: `codex mcp add dbforge --url ${url} --bearer-token-env-var ${TOKEN_VARIABLE}`,
        },
        { kind: 'profile', text: `export ${TOKEN_VARIABLE}=${token}` },
      ]
    case 'opencode': {
      const config = {
        $schema: 'https://opencode.ai/config.json',
        // The query tool may take a minute; OpenCode's default wait is 30 seconds.
        mcp: {
          dbforge: {
            type: 'remote',
            url,
            enabled: true,
            oauth: false,
            timeout: 70000,
            headers: { Authorization: bearer },
          },
        },
      }
      return [{ kind: 'config', file: OPENCODE_CONFIG, text: JSON.stringify(config, null, 2) }]
    }
    case 'pi':
      return [
        { kind: 'terminal', text: `pi mcp add dbforge --url ${url} --header "Authorization=${bearer}"` },
      ]
    case 'other': {
      const config = { mcpServers: { dbforge: { url, headers: { Authorization: bearer } } } }
      return [
        {
          kind: 'connect',
          text: `URL: ${url}\nTransport: Streamable HTTP\nHeader: Authorization: ${bearer}`,
        },
        { kind: 'config', file: 'mcp.json', text: JSON.stringify(config, null, 2) },
      ]
    }
  }
}

/** Where `agent` reads a project's standing instructions. */
export function instructionsFile(agent: AgentId): string {
  return agent === 'claude' ? 'CLAUDE.md' : 'AGENTS.md'
}

/** How `agent` checks that the server is connected, when it has a command for it. */
function checkCommand(agent: AgentId): string | null {
  switch (agent) {
    case 'claude':
      return 'claude mcp get dbforge'
    case 'codex':
      return 'codex mcp list'
    case 'opencode':
      return 'opencode mcp list'
    case 'pi':
      return 'pi mcp list'
    case 'other':
      return null
  }
}

/** The note for a project's instructions file that has the agent use DBForge for its databases. */
export function projectNote(): string {
  return [
    '## Databases',
    '',
    'Query and change this project’s databases through the `dbforge` MCP server, not with',
    'database clients or credentials: DBForge holds the connections and their passwords.',
    'Start with `list_connections`; each connection says its language and whether you may',
    'change data. DBForge must be open for the server to answer.',
  ].join('\n')
}

/** A prompt that has `agent` add DBForge itself, then say so in the project's instructions. */
export function agentPrompt(agent: AgentId, url: string, token: string): string {
  const steps = setupSteps(agent, url, token)
  const check = checkCommand(agent)
  const how = steps
    .map((step) => {
      switch (step.kind) {
        case 'terminal':
          return `Run:\n\n${step.text}`
        case 'profile':
          return `Add this line to my shell profile (~/.zshrc or ~/.bashrc) and load it:\n\n${step.text}`
        case 'config':
          return `Merge this into ${step.file}, keeping what is already there:\n\n${step.text}`
        case 'connect':
          return `Its connection details:\n\n${step.text}`
      }
    })
    .join('\n\n')
  return [
    agent === 'other'
      ? 'Add the MCP server "dbforge" (the DBForge database app on this computer) to your MCP configuration, available in every project.'
      : 'Add the MCP server "dbforge" (the DBForge database app on this computer) to your configuration, available in every project.',
    how,
    check
      ? `Then check it is connected with \`${check}\` (DBForge must be open), and tell me if it isn't.`
      : 'Then check it is connected (DBForge must be open), and tell me if it isn’t.',
    `Finally, add this section to the ${instructionsFile(agent)} of the current project, creating the file if needed:\n\n${projectNote()}`,
    'The token is a secret: don’t print it back, and don’t put it in the project.',
  ].join('\n\n')
}
