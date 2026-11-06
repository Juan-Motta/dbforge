import { MySQL, PostgreSQL, sql } from '@codemirror/lang-sql'
import type { SqlDialect } from '../../domain/connection'

/** SQL highlighting and completion for a dialect. */
export function sqlLanguage(dialect: SqlDialect = 'postgres') {
  return sql({ dialect: dialect === 'mysql' ? MySQL : PostgreSQL, upperCaseKeywords: true })
}
