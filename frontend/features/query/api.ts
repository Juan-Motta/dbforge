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

export const queryApi = {
  /** Runs a script; `database` defaults to the connection's own. */
  run: (connectionId: string, database: string | undefined, sql: string) =>
    invoke<QueryOutcome>('run_query', { connectionId, database, sql }),
}
