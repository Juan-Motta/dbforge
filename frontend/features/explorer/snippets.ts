import type { SqlDialect } from '../../domain/connection'
import type { TableRef } from '../../domain/table'
import { quote } from '../table-data/rowFilter'

/** Rows a "New query" from the explorer asks for. */
const PREVIEW_ROWS = 100

/**
 * A table as SQL names it from a console opened on its database. Schemaless
 * engines (MySQL) and SQLite's `main` need no schema.
 */
export function qualifiedName(dialect: SqlDialect, table: TableRef): string {
  if (dialect === 'mysql' || (dialect === 'sqlite' && table.schema === 'main'))
    return quote(dialect, table.name)
  return `${quote(dialect, table.schema)}.${quote(dialect, table.name)}`
}

/** The first rows of a table, as the engine's console writes it. */
export function previewQuery(dialect: SqlDialect, table: TableRef): string {
  switch (dialect) {
    case 'mongodb':
      return `db.getCollection(${JSON.stringify(table.name)}).find({}).limit(${PREVIEW_ROWS})`
    case 'cosmosdb':
      // The console runs on the container it was opened with.
      return `SELECT TOP ${PREVIEW_ROWS} * FROM c`
    case 'sqlserver':
      return `SELECT TOP ${PREVIEW_ROWS} * FROM ${qualifiedName(dialect, table)};`
    default:
      return `SELECT * FROM ${qualifiedName(dialect, table)} LIMIT ${PREVIEW_ROWS};`
  }
}

/** Every column, one per line, ready to trim. Relational dialects only. */
export function selectStatement(dialect: SqlDialect, table: TableRef, columns: string[]): string {
  const list = columns.map((column) => `  ${quote(dialect, column)}`).join(',\n')
  return `SELECT\n${list}\nFROM ${qualifiedName(dialect, table)};`
}

/** An INSERT naming every column, with NULL in each value to fill in. Relational dialects only. */
export function insertStatement(dialect: SqlDialect, table: TableRef, columns: string[]): string {
  const names = columns.map((column) => quote(dialect, column)).join(', ')
  const values = columns.map(() => 'NULL').join(', ')
  return `INSERT INTO ${qualifiedName(dialect, table)} (${names})\nVALUES (${values});`
}
