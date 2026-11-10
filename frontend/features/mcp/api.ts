import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export interface McpSettings {
  enabled: boolean
  port: number
}

export type McpStatus =
  { state: 'stopped' } | { state: 'running'; url: string } | { state: 'failed'; error: string }

export interface McpState {
  settings: McpSettings
  status: McpStatus
}

/** A change an agent wants to make on a connection whose tag asks before changes. */
export interface ApprovalRequest {
  id: string
  /** The agent's name for itself, e.g. `claude-code`. */
  agent: string | null
  connectionId: string
  connectionName: string
  tag: string | null
  database: string | null
  statement: string
}

export const mcpApi = {
  state: () => invoke<McpState>('mcp_state'),
  /** Saves the settings and starts, restarts or stops the server to match. */
  setSettings: (settings: McpSettings) => invoke<McpState>('set_mcp_settings', { settings }),
  token: () => invoke<string>('mcp_token'),
  regenerateToken: () => invoke<string>('regenerate_mcp_token'),
  /** False when the request had already timed out. */
  answer: (id: string, approved: boolean) => invoke<boolean>('answer_mcp_approval', { id, approved }),
  onApproval: (handler: (request: ApprovalRequest) => void) =>
    listen<ApprovalRequest>('mcp-approval', (event) => handler(event.payload)),
  /** A request nobody waits on any more: the agent left, or time ran out. */
  onWithdrawn: (handler: (id: string) => void) =>
    listen<string>('mcp-approval-withdrawn', (event) => handler(event.payload)),
}
