import type { ImportFormat } from './api'

/** The format a file's name suggests; CSV when it suggests none. */
export function formatOfPath(path: string): ImportFormat {
  const extension = path.split('.').pop()?.toLowerCase()
  if (extension === 'tsv') return 'tsv'
  if (extension === 'json' || extension === 'ndjson' || extension === 'jsonl') return 'json'
  return 'csv'
}

/**
 * The column each field goes into: the one of the same name, or failing that
 * the same name ignoring case, or none. Exact names are matched first, so a
 * field named like a column always gets it; a column is filled from one field.
 */
export function mapByName(fields: string[], columns: string[]): (string | null)[] {
  const mapping: (string | null)[] = fields.map((field) => (columns.includes(field) ? field : null))
  const taken = new Set(mapping.filter((column) => column !== null))
  return mapping.map((column, i) => {
    if (column !== null) return column
    const loose = columns.find((c) => !taken.has(c) && c.toLowerCase() === fields[i].toLowerCase())
    if (!loose) return null
    taken.add(loose)
    return loose
  })
}

/** Columns that more than one field goes into: the import would refuse them. */
export function repeatedColumns(mapping: (string | null)[]): string[] {
  const seen = new Set<string>()
  const repeated = new Set<string>()
  for (const column of mapping) {
    if (column === null) continue
    if (seen.has(column)) repeated.add(column)
    seen.add(column)
  }
  return [...repeated]
}
