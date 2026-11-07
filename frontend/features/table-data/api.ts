import { invoke } from '@tauri-apps/api/core'
import type { TableRef } from '../../domain/table'
import type { RowChange } from './edits'
import type { RowFilter } from './rowFilter'

export interface Column {
  name: string
  /** As the server writes it, e.g. `character varying(80)`. */
  dataType: string
  primaryKey: boolean
  /** Computed by the database (generated column, or identity GENERATED ALWAYS); not editable. */
  generated?: boolean
}

export interface RowsPage {
  columns: Column[]
  /** Values in the server's text form; `null` is SQL NULL. */
  rows: (string | null)[][]
  hasMore: boolean
}

export interface ApplySummary {
  updated: number
  inserted: number
  deleted: number
}

export const tableDataApi = {
  /** One page of rows, narrowed and ordered by `filter`. */
  fetchRows: (connectionId: string, table: TableRef, filter: RowFilter, offset: number, limit: number) =>
    invoke<RowsPage>('fetch_rows', { connectionId, table, filter, offset, limit }),
  /** Applies every change in one transaction, or none if any row changed since it was loaded. */
  applyChanges: (connectionId: string, table: TableRef, changes: RowChange[]) =>
    invoke<ApplySummary>('apply_changes', { connectionId, table, changes }),
  /** The SQL `applyChanges` would run, with values written in. */
  previewChanges: (connectionId: string, table: TableRef, changes: RowChange[]) =>
    invoke<string[]>('preview_changes', { connectionId, table, changes }),
}
