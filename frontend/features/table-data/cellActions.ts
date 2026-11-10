import type { SqlDialect } from '../../domain/connection'
import type { TableRef } from '../../domain/table'
import { qualifiedName } from '../explorer/snippets'
import { literal } from './foreignLinks'
import { orderText, quote, type RowFilter } from './rowFilter'

type Value = string | null

/** How a cell's menu narrows the rows by its value. */
export type ValueTest = 'equal' | 'notEqual' | 'isNull' | 'isNotNull'

/**
 * Engines whose filter is a condition on columns. MongoDB's is a document,
 * and a cell's text doesn't say whether its value is a number, a string or an
 * ObjectId, so it gets no filter from a value.
 */
export function filtersByValue(dialect: SqlDialect): boolean {
  return dialect !== 'mongodb'
}

/** The condition that keeps the rows whose `column` passes `test` against `value`. */
export function valueCondition(dialect: SqlDialect, column: string, test: ValueTest, value: Value): string {
  const name = quote(dialect, column)
  if (test === 'isNull' || (test === 'equal' && value === null)) {
    return dialect === 'cosmosdb' ? `IS_NULL(${name})` : `${name} IS NULL`
  }
  if (test === 'isNotNull' || (test === 'notEqual' && value === null)) {
    return dialect === 'cosmosdb' ? `NOT IS_NULL(${name})` : `${name} IS NOT NULL`
  }
  const text = literal(dialect, value ?? '')
  return `${name} ${test === 'equal' ? '=' : '<>'} ${text}`
}

/** `current` narrowed by `condition`, keeping what it already filtered. */
export function narrowedBy(current: RowFilter, condition: string): RowFilter {
  const existing = current.filter.trim()
  return { ...current, filter: existing ? `(${existing}) AND ${condition}` : condition }
}

/** `current` sorted by `column` alone. */
export function sortedBy(
  dialect: SqlDialect,
  column: string,
  direction: 'asc' | 'desc',
  current: RowFilter,
): RowFilter {
  return { ...current, order: orderText(dialect, column, direction) }
}

/** A row as a JSON object, column by column; NULL is `null`. */
export function rowAsJson(columns: string[], row: Value[]): string {
  return JSON.stringify(Object.fromEntries(columns.map((column, i) => [column, row[i]])), null, 2)
}

/** The INSERT that would add `row` again, with its values. Relational dialects only. */
export function rowAsInsert(dialect: SqlDialect, table: TableRef, columns: string[], row: Value[]): string {
  const names = columns.map((column) => quote(dialect, column)).join(', ')
  const values = row.map((value) => (value === null ? 'NULL' : literal(dialect, value))).join(', ')
  return `INSERT INTO ${qualifiedName(dialect, table)} (${names})\nVALUES (${values});`
}
