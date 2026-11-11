import { javascript } from '@codemirror/lang-javascript'
import { json } from '@codemirror/lang-json'
import { MSSQL, MySQL, PostgreSQL, SQLite, StandardSQL, sql, type SQLNamespace } from '@codemirror/lang-sql'
import type { SqlDialect } from '../../domain/connection'

/** Names the editor completes besides keywords: schemas, tables and columns. */
export interface SqlNames {
  namespace: SQLNamespace
  /** Its tables complete without the schema's name. */
  defaultSchema?: string
}

/** Highlighting and completion for a dialect; mongosh scripts are JavaScript. */
export function sqlLanguage(dialect: SqlDialect = 'postgres', names?: SqlNames) {
  if (dialect === 'mongodb') return javascript()
  const dialects = {
    postgres: PostgreSQL,
    mysql: MySQL,
    sqlite: SQLite,
    sqlserver: MSSQL,
    cosmosdb: StandardSQL,
  }
  return sql({
    dialect: dialects[dialect],
    upperCaseKeywords: true,
    schema: names?.namespace,
    defaultSchema: names?.defaultSchema,
  })
}

/** The language of a table's definition script: Cosmos DB defines a container in JSON. */
export function definitionLanguage(dialect: SqlDialect = 'postgres') {
  return dialect === 'cosmosdb' ? json() : sqlLanguage(dialect)
}
