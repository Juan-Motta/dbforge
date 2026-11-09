import type { DumpContent } from './api'

/** What a dump can hold, as the export dialog offers it. */
export const CONTENTS: { id: DumpContent; label: string; description: string }[] = [
  { id: 'all', label: 'Structure and data', description: 'The whole database' },
  { id: 'structure', label: 'Structure only', description: 'Schemas, tables, views, functions…' },
  { id: 'data', label: 'Data only', description: 'Rows, for tables that already exist' },
]

/** What restoring into `name` will do, given the server's databases. */
export function targetNote(name: string, databases: string[] | null): string {
  const trimmed = name.trim()
  if (trimmed === '') return 'Name the database to restore into.'
  if (databases === null) return 'Restores into this database, creating it if it doesn’t exist.'
  return databases.includes(trimmed)
    ? 'Restores into this existing database.'
    : 'Creates this database, then restores into it.'
}
