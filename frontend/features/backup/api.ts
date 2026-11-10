import { invoke } from '@tauri-apps/api/core'
import { open, save } from '@tauri-apps/plugin-dialog'

/** The window's own preferences, by the key the backup file keeps them under. */
export type Preferences = Partial<Record<'theme' | 'language' | 'exportFolder', string>>

export interface Exported {
  connections: number
  passwords: number
}

export interface BackupPreview {
  /** Milliseconds since the Unix epoch. */
  exportedAt: number
  appVersion: string
  preferences: Preferences
  newConnections: string[]
  updatedConnections: string[]
  passwords: number
  tags: number
  mcp: { enabled: boolean; port: number } | null
  mcpToken: boolean
  toolPaths: number
}

export interface ImportParts {
  connections: boolean
  mcp: boolean
  toolPaths: boolean
}

export interface Imported {
  added: number
  updated: number
  withoutPassword: string[]
  missingTools: string[]
}

export const backupApi = {
  export: (path: string, preferences: Preferences, secrets: boolean) =>
    invoke<Exported>('export_backup', { path, preferences, secrets }),
  read: (path: string) => invoke<BackupPreview>('read_backup', { path }),
  import: (path: string, parts: ImportParts) => invoke<Imported>('import_backup', { path, parts }),
}

/** Where to write the backup, with the native dialog; `null` when the user cancels. */
export function chooseBackupTarget(defaultName: string): Promise<string | null> {
  return save({ defaultPath: defaultName, filters: [{ name: 'DBForge backup', extensions: ['json'] }] })
}

/** A backup to read, with the native dialog; `null` when the user cancels. */
export async function chooseBackupFile(): Promise<string | null> {
  const picked = await open({ multiple: false, filters: [{ name: 'DBForge backup', extensions: ['json'] }] })
  return typeof picked === 'string' ? picked : null
}
