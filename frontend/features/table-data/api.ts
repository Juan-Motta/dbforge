import { invoke } from '@tauri-apps/api/core'
import type { TableRef } from '../../domain/table'

export interface Column {
  name: string
  /** As the server writes it, e.g. `character varying(80)`. */
  dataType: string
  primaryKey: boolean
}

export interface RowsPage {
  columns: Column[]
  /** Values in the server's text form; `null` is SQL NULL. */
  rows: (string | null)[][]
  hasMore: boolean
}

export const tableDataApi = {
  fetchRows: (connectionId: string, table: TableRef, offset: number, limit: number) =>
    invoke<RowsPage>('fetch_rows', { connectionId, table, offset, limit }),
}
