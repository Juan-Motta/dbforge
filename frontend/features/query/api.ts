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
  /** Why the script stopped before its end, after the statements in `results` ran. */
  error: string | null
}

/**
 * Where a console's queries run: a database (the connection's own when
 * omitted) and, for Cosmos DB, the container picked in the console.
 */
export interface QueryScope {
  database?: string
  container?: string
}

/**
 * Each console has a session: a connection the backend keeps open between runs,
 * so transactions and SET carry over. It opens on the first run.
 */
export const queryApi = {
  /** Runs a script in the session; another scope opens a new one. */
  run: (sessionId: string, connectionId: string, scope: QueryScope, sql: string) =>
    invoke<QueryOutcome>('run_query', { sessionId, connectionId, scope, sql }),
  /**
   * The plan of one statement as EXPLAIN's JSON. With `analyze` it really runs,
   * on its own connection and inside a transaction that is always rolled back.
   */
  explain: (connectionId: string, scope: QueryScope, sql: string, analyze: boolean) =>
    invoke<unknown>('explain_query', { connectionId, scope, sql, analyze }),
  /** Stops the running query; its run then fails with a cancel error. */
  cancel: (sessionId: string) => invoke<void>('cancel_query', { sessionId }),
  /** Closes the connection; uncommitted work is rolled back. */
  closeSession: (sessionId: string) => invoke<void>('close_session', { sessionId }),
}
