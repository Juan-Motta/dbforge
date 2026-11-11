import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { save } from '@tauri-apps/plugin-dialog'
import type { RowFilter } from '../table-data/rowFilter'
import type { GridColumn } from '../../components/data-grid/DataGrid'
import type { TableRef } from '../../domain/table'
import { defaultFileName, extensionOf, FORMATS, type ExportFormat, type FormatOptions } from './formats'

/** Rows the UI already holds, such as a query result. */
export interface RowsSource {
  /** Used for the file name and as the table in SQL INSERT. */
  name: string
  columns: GridColumn[]
  rows: (string | null)[][]
}

/** A format's options, and the columns to write when only some are (every one when absent). */
export type ExportOptions = FormatOptions & { columns?: string[] }

export const exportApi = {
  /** The rows as text in `format`, e.g. for the clipboard. */
  formatRows: (format: ExportFormat, options: ExportOptions, source: RowsSource) =>
    invoke<string>('format_rows', { format, options, ...source }),
  /** Saves the rows to `path`; returns how many were written. */
  exportRows: (path: string, format: ExportFormat, options: ExportOptions, source: RowsSource) =>
    invoke<number>('export_rows', { path, format, options, ...source }),
  /**
   * Streams the rows of a table the filter keeps, in its order, to `path`, as
   * job `jobId`; returns how many were written. A cancelled or failed export
   * leaves a file already at `path` as it was.
   */
  exportTable: (request: {
    jobId: string
    connectionId: string
    table: TableRef
    filter: RowFilter | null
    format: ExportFormat
    options: ExportOptions
    path: string
  }) => invoke<number>('export_table', request),
  /** Stops a running export. */
  cancel: (jobId: string) => invoke<void>('cancel_job', { jobId }),
  /** How many rows job `jobId` has written, as it goes; resolves to the function that stops listening. */
  onProgress: (jobId: string, handler: (rows: number) => void) =>
    listen<{ jobId: string; rows: number }>('export-progress', (event) => {
      if (event.payload.jobId === jobId) handler(event.payload.rows)
    }),
  /** Shows the file in Finder, or the system's file manager. */
  reveal: (path: string) => invoke<void>('reveal_file', { path }),
}

/** Asks where to save with the native dialog; `null` when the user cancels. */
export function chooseFile(name: string, format: ExportFormat): Promise<string | null> {
  const label = FORMATS.find((f) => f.id === format)!.label
  return save({
    defaultPath: defaultFileName(name, format),
    filters: [{ name: label, extensions: [extensionOf(format)] }],
  })
}
