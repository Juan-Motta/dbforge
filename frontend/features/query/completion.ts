import type { SQLNamespace } from '@codemirror/lang-sql'
import { ENGINE_LIST, ENGINES, type SqlDialect } from '../../domain/connection'

/** A database's tables and views with their columns, as the backend reads them for completion. */
export interface CompletionSchema {
  database: string
  tables: {
    schema: string
    name: string
    kind: 'table' | 'view'
    columns: { name: string; dataType: string }[]
  }[]
}

/** One name offered: what it is (`type` picks its icon) and a detail beside it. */
interface Option {
  label: string
  type: 'namespace' | 'type' | 'property'
  detail: string
}

/** The names the SQL editor completes besides keywords, and the schema typed without its name. */
export interface CompletionNames {
  namespace: SQLNamespace
  defaultSchema?: string
}

/**
 * Schemas, then their tables and views, then their columns, each with what it
 * is: `public.orders.total`, and `orders.total` in the default schema (the
 * engine's own, or for MySQL the database, which is its only schema).
 */
export function completionNames(schema: CompletionSchema, dialect: SqlDialect): CompletionNames {
  const namespace: Record<string, { self: Option; children: Record<string, SQLNamespace> }> = {}
  for (const table of schema.tables) {
    namespace[table.schema] ??= {
      self: { label: table.schema, type: 'namespace', detail: 'schema' },
      children: {},
    }
    namespace[table.schema].children[table.name] = {
      self: { label: table.name, type: 'type', detail: table.kind },
      children: table.columns.map((column) => ({
        label: column.name,
        type: 'property',
        detail: column.dataType,
      })),
    }
  }
  const engine = ENGINE_LIST.find((e) => ENGINES[e].dialect === dialect)
  const defaultSchema = (engine && ENGINES[engine].defaultSchema) || schema.database
  return { namespace, defaultSchema: defaultSchema && defaultSchema in namespace ? defaultSchema : undefined }
}

/** Whether `sql` may change which tables or columns exist, so completion reads them again. */
export function changesStructure(sql: string): boolean {
  return /\b(create|alter|drop|rename)\b/i.test(sql)
}
