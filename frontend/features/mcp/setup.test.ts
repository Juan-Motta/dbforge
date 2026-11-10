import { describe, expect, it } from 'vitest'
import { claudeCommand, codexCommand, codexToken, parsePort, serverUrl } from './setup'

describe('agent setup', () => {
  it('writes the commands that add DBForge to Claude Code and Codex', () => {
    const url = serverUrl(7419)
    expect(url).toBe('http://127.0.0.1:7419/mcp')
    expect(claudeCommand(url, 'abc')).toBe(
      'claude mcp add --transport http dbforge http://127.0.0.1:7419/mcp --header "Authorization: Bearer abc"',
    )
    expect(codexCommand(url)).toBe(
      'codex mcp add dbforge --url http://127.0.0.1:7419/mcp --bearer-token-env-var DBFORGE_MCP_TOKEN',
    )
    expect(codexToken('abc')).toBe('export DBFORGE_MCP_TOKEN=abc')
  })

  it('takes only unprivileged ports', () => {
    expect(parsePort(' 8080 ')).toBe(8080)
    expect(parsePort('80')).toBeNull()
    expect(parsePort('70000')).toBeNull()
    expect(parsePort('80a')).toBeNull()
  })
})
