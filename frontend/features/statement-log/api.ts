import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

/** What a statement was sent for. */
export type Origin = 'console' | 'browse' | 'edit' | 'metadata' | 'admin' | 'export' | 'dump' | 'connect'

/** One statement DBForge sent to a database, as the backend recorded it. */
export interface StatementEntry {
  /** Increases with every statement. */
  id: number
  /** When it started, in milliseconds since the Unix epoch. */
  at: number
  connectionId: string
  database: string | null
  origin: Origin
  /** In the engine's own language: SQL, a mongosh call, a redis-cli line. */
  text: string
  elapsedMs: number
  /** Rows returned, or changed when none came back; `null` when the engine doesn't say. */
  rows: number | null
  error: string | null
}

export const statementLogApi = {
  /** The statements recorded so far, oldest first. */
  list: () => invoke<StatementEntry[]>('list_statements'),
  clear: () => invoke<void>('clear_statements'),
  /** Each statement as it is recorded; resolves to the function that stops listening. */
  onLogged: (handler: (entry: StatementEntry) => void) =>
    listen<StatementEntry>('statement-logged', (event) => handler(event.payload)),
}
