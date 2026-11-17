import { describe, expect, it } from 'vitest'
import {
  agentPrompt,
  instructionsFile,
  parsePort,
  projectNote,
  serverUrl,
  setupSteps as steps,
  shellArgument,
  type AgentId,
} from './setup'

const url = serverUrl(7419)
const http = { transport: 'http', url, token: 'abc' } as const
const setupSteps = (agent: AgentId, _url: string, token: string) => steps(agent, { ...http, token })
const mac = { transport: 'stdio', executable: '/Applications/DBForge.app/Contents/MacOS/DBForge' } as const

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
    const prompt = agentPrompt('codex', http)
    expect(prompt).toContain(setupSteps('codex', url, 'abc')[0].text)
    expect(prompt).toContain('export DBFORGE_MCP_TOKEN=abc')
    expect(prompt).toContain('codex mcp list')
    expect(prompt).toContain('AGENTS.md')
    expect(prompt).toContain(projectNote())
    expect(instructionsFile('claude')).toBe('CLAUDE.md')
    expect(agentPrompt('other', http)).not.toContain('mcp list')
    // Over HTTP the wording stays as it was before stdio existed.
    expect(agentPrompt('claude', http)).toContain(
      "Then check it is connected with `claude mcp get dbforge` (DBForge must be open), and tell me if it isn't.",
    )
    expect(agentPrompt('other', http)).toContain(
      'Then check it is connected (DBForge must be open), and tell me if it isn’t.',
    )
  })

  it('has each agent start DBForge itself, with no token, on every platform', () => {
    expect(steps('claude', mac).map((step) => step.text)).toEqual([
      'claude mcp add --scope user dbforge -- /Applications/DBForge.app/Contents/MacOS/DBForge --mcp-stdio',
    ])
    expect(steps('codex', mac).map((step) => step.text)).toEqual([
      'codex mcp add dbforge -- /Applications/DBForge.app/Contents/MacOS/DBForge --mcp-stdio',
    ])
    expect(steps('pi', mac)[0].text).toBe(
      'pi mcp add dbforge -- /Applications/DBForge.app/Contents/MacOS/DBForge --mcp-stdio',
    )
    const [opencode] = steps('opencode', mac)
    expect(JSON.parse(opencode.text).mcp.dbforge).toMatchObject({
      type: 'local',
      command: [mac.executable, '--mcp-stdio'],
      enabled: true,
    })
    const other = steps('other', mac)
    expect(other[0].kind).toBe('launch')
    expect(JSON.parse(other[1].text)).toEqual({
      mcpServers: { dbforge: { command: mac.executable, args: ['--mcp-stdio'] } },
    })
    const prompt = agentPrompt('claude', mac)
    expect(prompt).toContain(steps('claude', mac)[0].text)
    expect(prompt).not.toContain('token')
  })

  it('quotes an executable path the way its own system’s shells read it', () => {
    expect(shellArgument('/usr/bin/dbforge')).toBe('/usr/bin/dbforge')
    expect(shellArgument('C:\\Program Files\\DBForge\\DBForge.exe')).toBe(
      '"C:\\Program Files\\DBForge\\DBForge.exe"',
    )
    expect(shellArgument("/home/ana/My Apps/DBForge's.AppImage")).toBe(
      "'/home/ana/My Apps/DBForge'\\''s.AppImage'",
    )
    const windows = {
      transport: 'stdio',
      executable: 'C:\\Users\\Ana\\AppData\\Local\\DBForge\\DBForge.exe',
    } as const
    expect(steps('claude', windows)[0].text).toBe(
      'claude mcp add --scope user dbforge -- C:\\Users\\Ana\\AppData\\Local\\DBForge\\DBForge.exe --mcp-stdio',
    )
  })
})
