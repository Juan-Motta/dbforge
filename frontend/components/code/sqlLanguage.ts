import { MSSQL, MySQL, PostgreSQL, SQLite, sql } from '@codemirror/lang-sql'
import type { SqlDialect } from '../../domain/connection'

/** SQL highlighting and completion for a dialect. */
export function sqlLanguage(dialect: SqlDialect = 'postgres') {
  const dialects = { postgres: PostgreSQL, mysql: MySQL, sqlite: SQLite, sqlserver: MSSQL }
  return sql({ dialect: dialects[dialect], upperCaseKeywords: true })
}
