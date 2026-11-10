import { describe, expect, it } from 'vitest'
import { agentPrompt, instructionsFile, parsePort, projectNote, serverUrl, setupSteps } from './setup'

const url = serverUrl(7419)

describe('agent setup', () => {
  it('serves on the loopback address and takes only unprivileged ports', () => {
    expect(url).toBe('http://127.0.0.1:7419/mcp')
    expect(parsePort(' 8080 ')).toBe(8080)
    expect(parsePort('80')).toBeNull()
    expect(parsePort('70000')).toBeNull()
    expect(parsePort('80a')).toBeNull()
  })

  it('writes each agent’s own way of adding a server for every project', () => {
    expect(setupSteps('claude', url, 'abc')[0].text).toBe(
      'claude mcp add --transport http --scope user dbforge http://127.0.0.1:7419/mcp --header "Authorization: Bearer abc"',
    )
    expect(setupSteps('codex', url, 'abc').map((step) => step.text)).toEqual([
      'codex mcp add dbforge --url http://127.0.0.1:7419/mcp --bearer-token-env-var DBFORGE_MCP_TOKEN',
      'export DBFORGE_MCP_TOKEN=abc',
    ])
    expect(setupSteps('pi', url, 'abc')[0].text).toBe(
      'pi mcp add dbforge --url http://127.0.0.1:7419/mcp --header "Authorization=Bearer abc"',
    )
    const [opencode] = setupSteps('opencode', url, 'abc')
    expect(opencode.file).toBe('~/.config/opencode/opencode.json')
    expect(JSON.parse(opencode.text).mcp.dbforge).toMatchObject({
      type: 'remote',
      url,
      oauth: false,
      headers: { Authorization: 'Bearer abc' },
    })
    const other = setupSteps('other', url, 'abc')
    expect(JSON.parse(other[1].text)).toEqual({
      mcpServers: { dbforge: { url, headers: { Authorization: 'Bearer abc' } } },
    })
  })

  it('prompts the agent to add the server, check it, and note it in the project’s instructions', () => {
    const prompt = agentPrompt('codex', url, 'abc')
    expect(prompt).toContain(setupSteps('codex', url, 'abc')[0].text)
    expect(prompt).toContain('export DBFORGE_MCP_TOKEN=abc')
    expect(prompt).toContain('codex mcp list')
    expect(prompt).toContain('AGENTS.md')
    expect(prompt).toContain(projectNote())
    expect(instructionsFile('claude')).toBe('CLAUDE.md')
    expect(agentPrompt('other', url, 'abc')).not.toContain('mcp list')
  })
})
