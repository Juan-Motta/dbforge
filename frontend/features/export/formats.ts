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
