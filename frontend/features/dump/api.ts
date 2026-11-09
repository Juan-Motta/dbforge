import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open, save } from '@tauri-apps/plugin-dialog'

/** What a dump holds. */
export type DumpContent = 'all' | 'structure' | 'data'

export interface DumpOptions {
  content: DumpContent
  /** Drops each object before creating it, so the file restores over an existing database. */
  clean: boolean
  /** Keeps owners and privileges, which then need the same roles where the file is restored. */
  keepOwners: boolean
}

export interface Exported {
  bytes: number
  elapsedMs: number
}

export interface Imported {
  /** The database didn't exist and was created for the restore. */
  created: boolean
  elapsedMs: number
}

/** A whole PostgreSQL database to a .sql file with pg_dump, and back with psql. */
export const dumpApi = {
  exportDatabase: (request: {
    jobId: string
    connectionId: string
    database: string
    path: string
    options: DumpOptions
  }) => invoke<Exported>('export_database', { request }),
  importDatabase: (request: { jobId: string; connectionId: string; database: string; path: string }) =>
    invoke<Imported>('import_database', { request }),
  cancel: (jobId: string) => invoke<void>('cancel_dump', { jobId }),
  /** Each line the job's tool writes while it runs; resolves to the function that stops listening. */
  onProgress: (jobId: string, handler: (line: string) => void) =>
    listen<{ jobId: string; line: string }>('dump-progress', (event) => {
      if (event.payload.jobId === jobId) handler(event.payload.line)
    }),
}

const SQL_FILES = [{ name: 'SQL', extensions: ['sql'] }]

/** Where to write a dump, with the native dialog; `null` when the user cancels. */
export function chooseDumpFile(database: string): Promise<string | null> {
  return save({ defaultPath: `${database}.sql`, filters: SQL_FILES })
}

/** A .sql file to restore, with the native dialog; `null` when the user cancels. */
export function chooseSqlFile(): Promise<string | null> {
  return open({
    multiple: false,
    directory: false,
    filters: [...SQL_FILES, { name: 'All files', extensions: ['*'] }],
  })
}
