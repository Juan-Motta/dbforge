export type ExportFormat = 'csv' | 'tsv' | 'json' | 'sql' | 'markdown'

/** Each format's description is the `<id>Description` message. */
export const FORMATS: { id: ExportFormat; label: string; extension: string }[] = [
  { id: 'csv', label: 'CSV', extension: 'csv' },
  { id: 'tsv', label: 'TSV', extension: 'tsv' },
  { id: 'json', label: 'JSON', extension: 'json' },
  { id: 'sql', label: 'SQL INSERT', extension: 'sql' },
  { id: 'markdown', label: 'Markdown', extension: 'md' },
]

export function extensionOf(format: ExportFormat): string {
  return FORMATS.find((f) => f.id === format)!.extension
}

/** `orders.csv`; characters that file systems reject become `_`. */
export function defaultFileName(name: string, format: ExportFormat): string {
  const safe = name.replace(/[\\/:*?"<>|]+/g, '_').trim() || 'export'
  return `${safe}.${extensionOf(format)}`
}

export type JsonStyle = 'array' | 'lines'

/** What a format writes besides the values; each option matters only to the formats its comment names. */
export interface FormatOptions {
  /** CSV and TSV: a first line with the column names. */
  header: boolean
  /** CSV and TSV: what a NULL is written as. */
  nullText: string
  jsonStyle: JsonStyle
  /** SQL: how many rows each INSERT adds. */
  rowsPerInsert: number
}

export const DEFAULT_OPTIONS: FormatOptions = {
  header: true,
  nullText: '',
  jsonStyle: 'array',
  rowsPerInsert: 1,
}

/** The sizes offered for SQL; the server caps it at the largest. */
export const ROWS_PER_INSERT = [1, 100, 500, 1000] as const

/** The options sent with an export: the format's, and the columns when only some are chosen. */
export function exportOptions(options: FormatOptions, chosen: string[], all: string[]) {
  const some = chosen.length !== all.length
  return { ...options, columns: some ? all.filter((name) => chosen.includes(name)) : undefined }
}
