import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'

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

/** The pg_dump or psql an export or import runs. */
export interface ToolInfo {
  path: string
  /** `major.minor`. */
  version: string
  /** Chosen by the user rather than found by DBForge. */
  custom: boolean
}

export type DumpTool = 'pg_dump' | 'psql'

export interface Imported {
  /** The database didn't exist and was created for the restore. */
  created: boolean
  elapsedMs: number
  /** Extensions the file creates that the server doesn't have, so the import left them out. */
  skippedExtensions: string[]
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
  cancel: (jobId: string) => invoke<void>('cancel_job', { jobId }),
  /** Which tool will run; rejects with why none can. */
  tool: (name: DumpTool) => invoke<ToolInfo>('dump_tool', { name }),
  /** Runs `path` from now on once it proves to be that tool; `null` goes back to the one found. */
  setTool: (name: DumpTool, path: string | null) => invoke<ToolInfo>('set_dump_tool', { name, path }),
  fileExists: (path: string) => invoke<boolean>('dump_file_exists', { path }),
  /** The exports DBForge wrote in `folder`; other files there don't count. */
  folderUsage: (folder: string) => invoke<ExportFiles>('export_folder_usage', { folder }),
  /** Deletes the exports in `folder`, leaving its other files, and says what went. */
  clearFolder: (folder: string) => invoke<ExportFiles>('clear_export_folder', { folder }),
  /** Each line the job's tool writes while it runs; resolves to the function that stops listening. */
  onProgress: (jobId: string, handler: (line: string) => void) =>
    listen<{ jobId: string; line: string }>('dump-progress', (event) => {
      if (event.payload.jobId === jobId) handler(event.payload.line)
    }),
}

const SQL_FILES = [{ name: 'SQL', extensions: ['sql'] }]

/** A folder, with the native dialog; `null` when the user cancels. */
export function chooseFolder(start?: string): Promise<string | null> {
  return open({ directory: true, multiple: false, defaultPath: start })
}

/** A program to run in place of the tool found, with the native dialog; `null` when the user cancels. */
export function chooseToolFile(start?: string): Promise<string | null> {
  return open({ directory: false, multiple: false, defaultPath: start })
}

/** A .sql file to restore, with the native dialog; `null` when the user cancels. `allFiles` names the catch-all filter. */
export function chooseSqlFile(allFiles: string): Promise<string | null> {
  return open({
    multiple: false,
    directory: false,
    filters: [...SQL_FILES, { name: allFiles, extensions: ['*'] }],
  })
}

/** How many export files, and their size in bytes. */
export interface ExportFiles {
  count: number
  bytes: number
}
