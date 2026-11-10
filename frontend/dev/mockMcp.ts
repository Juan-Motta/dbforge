import { emit } from '@tauri-apps/api/event'
import type { ApprovalRequest, McpState } from '../features/mcp/api'

/** The MCP server as the mock backend runs it: settings only, nothing listens. */
let state: McpState = { settings: { enabled: false, port: 7419 }, status: { state: 'stopped' } }
let token = 'f3a9c1d27e4b4c6a8d0e5b7f9a1c3e5d7b9f1a3c5e7d9b1f3a5c7e9d1b3f5a7c'

export function mockMcp(
  cmd: string,
  payload: Record<string, unknown>,
): { handled: boolean; value?: unknown } {
  switch (cmd) {
    case 'mcp_state':
      return { handled: true, value: state }
    case 'set_mcp_settings': {
      const settings = payload.settings as McpState['settings']
      state = {
        settings,
        status: settings.enabled
          ? { state: 'running', url: `http://127.0.0.1:${settings.port}/mcp` }
          : { state: 'stopped' },
      }
      return { handled: true, value: state }
    }
    case 'mcp_token':
      return { handled: true, value: token }
    case 'regenerate_mcp_token':
      token = crypto.randomUUID().replaceAll('-', '') + crypto.randomUUID().replaceAll('-', '')
      return { handled: true, value: token }
    case 'answer_mcp_approval':
      return { handled: true, value: true }
    default:
      return { handled: false }
  }
}

/** For trying the approval dialog: `__mockApproval()` in the console. */
export function exposeMockApproval() {
  ;(window as unknown as { __mockApproval: () => void }).__mockApproval = () => {
    const request: ApprovalRequest = {
      id: crypto.randomUUID(),
      agent: 'claude-code',
      connectionId: 'demo-postgres',
      connectionName: 'demo-postgres',
      tag: 'PROD',
      database: 'shop',
      statement: "UPDATE public.orders SET status = 'refunded' WHERE id = 1042;",
    }
    void emit('mcp-approval', request)
  }
}
