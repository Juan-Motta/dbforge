import type { DumpContent } from './api'
import type { DumpMessages } from './messages'

/** What a dump can hold, as the export dialog offers it. */
export function contentsOf(m: DumpMessages): { id: DumpContent; label: string; description: string }[] {
  return [
    { id: 'all', label: m.allLabel, description: m.allDescription },
    { id: 'structure', label: m.structureLabel, description: m.structureDescription },
    { id: 'data', label: m.dataLabel, description: m.dataDescription },
  ]
}

/** What restoring into `name` will do, given the server's databases. */
export function targetNote(name: string, databases: string[] | null, m: DumpMessages): string {
  const trimmed = name.trim()
  if (trimmed === '') return m.noteNameTarget
  if (databases === null) return m.noteRestoreOrCreate
  return databases.includes(trimmed) ? m.noteRestoreExisting : m.noteCreateThenRestore
}
