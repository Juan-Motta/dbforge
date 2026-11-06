/**
 * Pending edits to a table, kept until the user applies or discards them.
 * Rows are identified by their primary key, so edits survive a reload of the page.
 */

type Value = string | null
type Values = Record<string, Value>

export interface Edits {
  /** Changed cells of loaded rows, by row key. */
  updates: Map<string, { key: Values; values: Values }>
  /** Rows marked for deletion, by row key. */
  deletes: Map<string, Values>
  /** New rows, newest first; only the columns the user set (the rest take their DEFAULT). */
  inserts: { id: number; values: Values }[]
}

export const NO_EDITS: Edits = { updates: new Map(), deletes: new Map(), inserts: [] }

/** What the backend's `apply_changes` expects. */
export type RowChange =
  | { kind: 'update'; key: Values; values: Values }
  | { kind: 'insert'; values: Values }
  | { kind: 'delete'; key: Values }

/** The primary key values of a row, by column name. */
export function keyOf(row: Value[], columns: { name: string; primaryKey: boolean }[]): Values {
  return Object.fromEntries(columns.flatMap((c, i) => (c.primaryKey ? [[c.name, row[i]]] : [])))
}

export function rowKey(key: Values): string {
  return JSON.stringify(Object.entries(key).sort(([a], [b]) => a.localeCompare(b)))
}

/** Sets a cell of a loaded row; setting it back to what was loaded drops the change. */
export function editCell(edits: Edits, key: Values, column: string, value: Value, original: Value): Edits {
  const id = rowKey(key)
  const current = edits.updates.get(id)?.values ?? {}
  const values = { ...current }
  if (value === original) delete values[column]
  else values[column] = value

  const updates = new Map(edits.updates)
  if (Object.keys(values).length === 0) updates.delete(id)
  else updates.set(id, { key, values })
  return { ...edits, updates }
}

/** Marks a loaded row for deletion, or unmarks it. Its cell edits are dropped either way. */
export function toggleDelete(edits: Edits, key: Values): Edits {
  const id = rowKey(key)
  const deletes = new Map(edits.deletes)
  const updates = new Map(edits.updates)
  if (deletes.has(id)) deletes.delete(id)
  else deletes.set(id, key)
  updates.delete(id)
  return { ...edits, deletes, updates }
}

export function addRow(edits: Edits): Edits {
  const id = Math.max(0, ...edits.inserts.map((r) => r.id)) + 1
  return { ...edits, inserts: [{ id, values: {} }, ...edits.inserts] }
}

export function editNewCell(edits: Edits, id: number, column: string, value: Value): Edits {
  return {
    ...edits,
    inserts: edits.inserts.map((row) =>
      row.id === id ? { ...row, values: { ...row.values, [column]: value } } : row,
    ),
  }
}

export function removeNewRow(edits: Edits, id: number): Edits {
  return { ...edits, inserts: edits.inserts.filter((row) => row.id !== id) }
}

export function counts(edits: Edits) {
  return { updated: edits.updates.size, inserted: edits.inserts.length, deleted: edits.deletes.size }
}

export function hasEdits(edits: Edits): boolean {
  const { updated, inserted, deleted } = counts(edits)
  return updated + inserted + deleted > 0
}

/**
 * The changes in the order they run: deletes first so a new row can reuse a
 * deleted row's unique value, then updates, then inserts oldest first.
 */
export function toChanges(edits: Edits): RowChange[] {
  return [
    ...[...edits.deletes.values()].map((key): RowChange => ({ kind: 'delete', key })),
    ...[...edits.updates.values()].map(({ key, values }): RowChange => ({ kind: 'update', key, values })),
    ...[...edits.inserts].reverse().map(({ values }): RowChange => ({ kind: 'insert', values })),
  ]
}
