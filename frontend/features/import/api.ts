import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import type { TableRef } from '../../domain/table'

export type ImportFormat = 'csv' | 'tsv' | 'json'

/** How a CSV or TSV file is read; JSON ignores these. */
export interface ReadOptions {
  /** The first line holds the column names. */
  header: boolean
  /** An unquoted value equal to this is NULL. */
  nullText: string
  /** CSV: what separates the values. */
  delimiter: string
}

export const DEFAULT_READ_OPTIONS: ReadOptions = { header: true, nullText: '', delimiter: ',' }

/** The file's fields, and its first rows (`null` is NULL). */
export interface FilePreview {
  fields: string[]
  rows: (string | null)[][]
}

export const importApi = {
  preview: (path: string, format: ImportFormat, options: ReadOptions) =>
    invoke<FilePreview>('preview_import', { path, format, options }),
  /**
   * Imports the file into the table as job `jobId`: each field into the column
   * at the same position of `columns`, or left out where it is `null`.
   * Resolves to how many rows went in.
   */
  importTable: (request: {
    jobId: string
    connectionId: string
    table: TableRef
    path: string
    format: ImportFormat
    options: ReadOptions
    fields: string[]
    columns: (string | null)[]
  }) => invoke<number>('import_table', { request }),
  cancel: (jobId: string) => invoke<void>('cancel_job', { jobId }),
  /** How many rows job `jobId` has imported, as it goes; resolves to the function that stops listening. */
  onProgress: (jobId: string, handler: (rows: number) => void) =>
    listen<{ jobId: string; rows: number }>('import-progress', (event) => {
      if (event.payload.jobId === jobId) handler(event.payload.rows)
    }),
}

/** A CSV, TSV or JSON file, with the native dialog; `null` when the user cancels. */
export function chooseDataFile(label: string): Promise<string | null> {
  return open({
    multiple: false,
    directory: false,
    filters: [{ name: label, extensions: ['csv', 'tsv', 'txt', 'json', 'ndjson', 'jsonl'] }],
  })
}
