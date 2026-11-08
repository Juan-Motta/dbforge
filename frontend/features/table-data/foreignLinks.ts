import type { SqlDialect } from '../../domain/connection'
import type { TableRef } from '../../domain/table'
import type { ForeignKey } from '../table-structure/api'
import { quote, type RowFilter } from './rowFilter'

/**
 * The table a foreign key points to. Engines name it bare when it is in the
 * same schema, else `schema.table`, with PostgreSQL's quotes where needed.
 */
export function referencedTable(from: TableRef, name: string): TableRef {
  const parts = splitQualified(name)
  return parts.length === 2
    ? { database: from.database, schema: parts[0], name: parts[1] }
    : { database: from.database, schema: from.schema, name: parts[parts.length - 1] }
}

/** `a.b` or `"a.x"."b"` into its parts, unquoted. */
function splitQualified(name: string): string[] {
  const parts: string[] = []
  let current = ''
  let quoted = false
  for (let i = 0; i < name.length; i++) {
    const c = name[i]
    if (c === '"' && quoted && name[i + 1] === '"') {
      current += '"'
      i++
    } else if (c === '"') {
      quoted = !quoted
    } else if (c === '.' && !quoted) {
      parts.push(current)
      current = ''
    } else {
      current += c
    }
  }
  return [...parts, current]
}

/**
 * The filter that shows the row a foreign key points to: each referenced
 * column equal to this row's value. `null` while any key column is NULL,
 * since such a row points nowhere.
 */
export function linkFilter(
  dialect: SqlDialect,
  key: ForeignKey,
  row: Record<string, string | null>,
): RowFilter | null {
  const conditions: string[] = []
  for (const [i, column] of key.columns.entries()) {
    const value = row[column]
    if (value === null || value === undefined) return null
    conditions.push(`${quote(dialect, key.referencedColumns[i])} = ${literal(dialect, value)}`)
  }
  return { filter: conditions.join(' AND '), order: '' }
}

/** A string literal; the server converts it to the column's type. */
function literal(dialect: SqlDialect, value: string): string {
  const escaped = dialect === 'mysql' ? value.replaceAll('\\', '\\\\') : value
  return `'${escaped.replaceAll("'", "''")}'`
}
