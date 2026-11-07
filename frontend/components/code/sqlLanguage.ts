import { javascript } from '@codemirror/lang-javascript'
import { MSSQL, MySQL, PostgreSQL, SQLite, sql } from '@codemirror/lang-sql'
import type { SqlDialect } from '../../domain/connection'

/** Highlighting and completion for a dialect; mongosh scripts are JavaScript. */
export function sqlLanguage(dialect: SqlDialect = 'postgres') {
  if (dialect === 'mongodb') return javascript()
  const dialects = { postgres: PostgreSQL, mysql: MySQL, sqlite: SQLite, sqlserver: MSSQL }
  return sql({ dialect: dialects[dialect], upperCaseKeywords: true })
}
