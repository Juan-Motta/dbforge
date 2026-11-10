import type { SqlDialect } from '../../domain/connection'
import type { messages } from './messages'

/**
 * What the filter bar narrows and orders a table's rows by. SQL engines read a
 * `WHERE` condition and an `ORDER BY` list as typed, Cosmos DB the same on its
 * items, called `c`; MongoDB reads a filter and a sort document in mongosh syntax.
 */
export interface RowFilter {
  filter: string
  order: string
}

export const NO_FILTER: RowFilter = { filter: '', order: '' }

export function isFiltered(filter: RowFilter): boolean {
  return filter.filter.trim() !== '' || filter.order.trim() !== ''
}

/** How the bar labels and hints its two fields for an engine, in the language of `m`. */
export function filterLabels(dialect: SqlDialect, m: typeof messages.en) {
  switch (dialect) {
    case 'mongodb':
      return {
        filter: m.filter,
        filterHint: "{status: 'paid', total: {$gt: 100}}",
        order: m.sort,
        orderHint: '{created: -1}',
      }
    case 'cosmosdb':
      return {
        filter: 'WHERE',
        filterHint: "c.status = 'paid' AND c.total > 100",
        order: 'ORDER BY',
        orderHint: 'c.created DESC',
      }
    default:
      return {
        filter: 'WHERE',
        filterHint: "status = 'paid' AND total > 100",
        order: 'ORDER BY',
        orderHint: 'created_at DESC',
      }
  }
}

/** A column name as the dialect writes it in SQL (a key in a MongoDB document, a property of Cosmos DB's `c`). */
export function quote(dialect: SqlDialect, column: string): string {
  switch (dialect) {
    case 'mysql':
      return '`' + column.replaceAll('`', '``') + '`'
    case 'sqlserver':
      return '[' + column.replaceAll(']', ']]') + ']'
    case 'mongodb':
      return /^[A-Za-z_$][\w$]*$/.test(column) ? column : JSON.stringify(column)
    case 'cosmosdb':
      return /^[A-Za-z_][\w]*$/.test(column) ? `c.${column}` : `c[${JSON.stringify(column)}]`
    default:
      return '"' + column.replaceAll('"', '""') + '"'
  }
}

function orderText(dialect: SqlDialect, column: string, direction: 'asc' | 'desc'): string {
  const name = quote(dialect, column)
  if (dialect === 'mongodb') return `{${name}: ${direction === 'asc' ? 1 : -1}}`
  return direction === 'asc' ? name : `${name} DESC`
}

/** Which way `order` sorts by `column` alone, as the header shows it; `null` otherwise. */
export function sortOf(dialect: SqlDialect, column: string, order: string): 'asc' | 'desc' | null {
  const text = order.trim()
  if (text === orderText(dialect, column, 'asc')) return 'asc'
  if (text === orderText(dialect, column, 'desc')) return 'desc'
  return null
}

/** A header click: ascending, then descending, then no order; it replaces any other order. */
export function sortBy(dialect: SqlDialect, column: string, current: RowFilter): RowFilter {
  const now = sortOf(dialect, column, current.order)
  const order =
    now === null ? orderText(dialect, column, 'asc') : now === 'asc' ? orderText(dialect, column, 'desc') : ''
  return { ...current, order }
}
