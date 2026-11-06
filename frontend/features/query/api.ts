import { invoke } from '@tauri-apps/api/core'
import type { GridColumn } from '../../components/data-grid/DataGrid'

export interface StatementResult {
  /** Empty for statements that return no rows (INSERT, CREATE, …). */
  columns: GridColumn[]
  rows: (string | null)[][]
  /** More rows came back than the backend keeps (500). */
  truncated: boolean
  /** Rows changed, or rows returned for a SELECT. */
  rowsAffected: number
}

export interface QueryOutcome {
  results: StatementResult[]
  elapsedMs: number
}

/**
 * Each console has a session: a connection the backend keeps open between runs,
 * so transactions and SET carry over. It opens on the first run.
 */
export const queryApi = {
  /** Runs a script; `database` defaults to the connection's own. */
  run: (sessionId: string, connectionId: string, database: string | undefined, sql: string) =>
    invoke<QueryOutcome>('run_query', { sessionId, connectionId, database, sql }),
  /** Stops the running query; its run then fails with a cancel error. */
  cancel: (sessionId: string) => invoke<void>('cancel_query', { sessionId }),
  /** Closes the connection; uncommitted work is rolled back. */
  closeSession: (sessionId: string) => invoke<void>('close_session', { sessionId }),
}
