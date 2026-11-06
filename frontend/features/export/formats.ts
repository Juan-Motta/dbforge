export type ExportFormat = 'csv' | 'tsv' | 'json' | 'sql' | 'markdown'

export const FORMATS: { id: ExportFormat; label: string; extension: string; description: string }[] = [
  { id: 'csv', label: 'CSV', extension: 'csv', description: 'Spreadsheets and tools' },
  { id: 'tsv', label: 'TSV', extension: 'tsv', description: 'Tab separated' },
  { id: 'json', label: 'JSON', extension: 'json', description: 'An array of objects' },
  { id: 'sql', label: 'SQL INSERT', extension: 'sql', description: 'Re-import anywhere' },
  { id: 'markdown', label: 'Markdown', extension: 'md', description: 'Docs and pull requests' },
]

export function extensionOf(format: ExportFormat): string {
  return FORMATS.find((f) => f.id === format)!.extension
}

/** `orders.csv`; characters that file systems reject become `_`. */
export function defaultFileName(name: string, format: ExportFormat): string {
  const safe = name.replace(/[\\/:*?"<>|]+/g, '_').trim() || 'export'
  return `${safe}.${extensionOf(format)}`
}
