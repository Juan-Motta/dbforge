/** Identifies a table or view in a connection. */
export interface TableRef {
  database: string
  schema: string
  name: string
}

/**
 * A table's explorer path as a `TableRef`: `[database, schema, name]`, or
 * `[database, name]` for engines without schemas (MySQL), where the database
 * plays the schema's part.
 */
export function tableRefFromPath(path: string[]): TableRef {
  if (path.length === 2) return { database: path[0], schema: path[0], name: path[1] }
  const [database, schema, name] = path
  return { database, schema, name }
}
