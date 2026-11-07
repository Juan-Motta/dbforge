import { invoke } from '@tauri-apps/api/core'
import type { RowFilter } from '../table-data/rowFilter'
import { save } from '@tauri-apps/plugin-dialog'
import type { GridColumn } from '../../components/data-grid/DataGrid'
import type { TableRef } from '../../domain/table'
import { defaultFileName, extensionOf, FORMATS, type ExportFormat } from './formats'

/** Rows the UI already holds, such as a query result. */
export interface RowsSource {
  /** Used for the file name and as the table in SQL INSERT. */
  name: string
  columns: GridColumn[]
  rows: (string | null)[][]
}

export const exportApi = {
  /** The rows as text in `format`, e.g. for the clipboard. */
  formatRows: (format: ExportFormat, source: RowsSource) =>
    invoke<string>('format_rows', { format, ...source }),
  /** Saves the rows to `path`; returns how many were written. */
  exportRows: (path: string, format: ExportFormat, source: RowsSource) =>
    invoke<number>('export_rows', { path, format, ...source }),
  /** Streams every row of a table to `path`; returns how many were written. */
  /** Every row the filter keeps, in its order. */
  exportTable: (
    connectionId: string,
    table: TableRef,
    filter: RowFilter,
    format: ExportFormat,
    path: string,
  ) => invoke<number>('export_table', { connectionId, table, filter, format, path }),
}

/** Asks where to save with the native dialog; `null` when the user cancels. */
export function chooseFile(name: string, format: ExportFormat): Promise<string | null> {
  const label = FORMATS.find((f) => f.id === format)!.label
  return save({
    defaultPath: defaultFileName(name, format),
    filters: [{ name: label, extensions: [extensionOf(format)] }],
  })
}
