import type { TableStructure } from './api'

export type KeyKind = 'PK' | 'FK' | 'UNI' | 'IDX'

/**
 * The keys each column takes part in, strongest first. Only plain column
 * indexes count; an index on an expression like `lower(email)` marks nothing.
 * A partial unique index only makes some rows unique, so it counts as IDX.
 */
export function columnKeys(structure: TableStructure): Map<string, KeyKind[]> {
  const keys = new Map<string, Set<KeyKind>>()
  const mark = (column: string, kind: KeyKind) => {
    if (!keys.has(column)) keys.set(column, new Set())
    keys.get(column)!.add(kind)
  }

  for (const index of structure.indexes) {
    const unique = index.unique && index.condition === null && index.columns.length === 1
    const kind = index.primary ? 'PK' : unique ? 'UNI' : 'IDX'
    index.columns.forEach((column) => mark(unquote(column), kind))
  }
  for (const key of structure.foreignKeys) key.columns.forEach((column) => mark(column, 'FK'))

  const order: KeyKind[] = ['PK', 'FK', 'UNI', 'IDX']
  return new Map([...keys].map(([column, kinds]) => [column, order.filter((kind) => kinds.has(kind))]))
}

/** Index columns come back as SQL, so mixed-case names arrive quoted: `"Note"`. */
function unquote(column: string): string {
  return /^".*"$/.test(column) ? column.slice(1, -1).replaceAll('""', '"') : column
}

const UNITS = ['B', 'kB', 'MB', 'GB', 'TB']

/** 0 B, 16 kB, 9.4 MB: one decimal below 10, whole numbers above. */
export function formatBytes(bytes: number): string {
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit++
  }
  const text = unit > 0 && value < 10 ? value.toFixed(1) : Math.round(value).toString()
  return `${text} ${UNITS[unit]}`
}

/** "~48,213 rows · 9.4 MB data · 3.1 MB indexes"; views have no size. */
export function summary(structure: TableStructure): string {
  const parts = []
  if (structure.rowEstimate !== null) parts.push(`~${structure.rowEstimate.toLocaleString('en-US')} rows`)
  if (structure.kind === 'table') {
    parts.push(`${formatBytes(structure.dataBytes)} data`, `${formatBytes(structure.indexBytes)} indexes`)
  }
  return parts.join(' · ')
}
