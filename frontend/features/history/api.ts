import { invoke } from '@tauri-apps/api/core'

export interface HistoryEntry {
  id: string
  /** Milliseconds since the Unix epoch. */
  executedAt: number
  connectionId: string
  /** The name when it ran; the connection may have been renamed or deleted since. */
  connectionName: string
  database: string | null
  /** The Cosmos DB container the console queried. */
  container?: string | null
  sql: string
  elapsedMs: number
  statements: number
  /** Rows of the last statement: returned by a SELECT, changed by anything else. */
  rows: number
  error: string | null
}

export const historyApi = {
  /**
   * Newest first; at most 500. `search` matches the SQL text, ignoring case;
   * with `connectionId`, only that connection's runs.
   */
  list: (search: string, connectionId: string | null, errorsOnly: boolean) =>
    invoke<HistoryEntry[]>('list_history', { search, connectionId, errorsOnly }),
}
