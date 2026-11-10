/** The environment variable Codex reads the token from: Codex keeps no secrets in its config. */
export const TOKEN_VARIABLE = 'DBFORGE_MCP_TOKEN'

/** Adds DBForge to Claude Code, the token sent as a header. */
export function claudeCommand(url: string, token: string): string {
  return `claude mcp add --transport http dbforge ${url} --header "Authorization: Bearer ${token}"`
}

/** Adds DBForge to Codex, which reads the token from the environment. */
export function codexCommand(url: string): string {
  return `codex mcp add dbforge --url ${url} --bearer-token-env-var ${TOKEN_VARIABLE}`
}

/** The line that puts the token where Codex reads it, for a shell profile. */
export function codexToken(token: string): string {
  return `export ${TOKEN_VARIABLE}=${token}`
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
