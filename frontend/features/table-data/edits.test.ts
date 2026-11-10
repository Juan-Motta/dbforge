import { describe, expect, it } from 'vitest'
import {
  addDocument,
  addRow,
  addRowWith,
  cellAt,
  counts,
  DEFAULT,
  editCell,
  editNewCell,
  editNewDocument,
  hasEdits,
  keyOf,
  NO_EDITS,
  removeNewDocument,
  removeNewRow,
  replaceDocument,
  rowKey,
  toChanges,
  toggleDelete,
  unsetNewCell,
} from './edits'

const columns = [
  { name: 'id', primaryKey: true },
  { name: 'name', primaryKey: false },
  { name: 'note', primaryKey: false },
]
const one = { id: '1' }

describe('keys', () => {
  it('takes the primary key columns of a row', () => {
    expect(keyOf(['1', 'Ann', null], columns)).toEqual({ id: '1' })
  })

  it('does not depend on column order', () => {
    expect(rowKey({ b: '2', a: '1' })).toBe(rowKey({ a: '1', b: '2' }))
  })
})

describe('cell edits', () => {
  it('records a changed cell, NULL included', () => {
    const edits = editCell(editCell(NO_EDITS, one, 'name', 'Bea', 'Ann'), one, 'note', null, 'hi')
    expect(toChanges(edits)).toEqual([{ kind: 'update', key: one, values: { name: 'Bea', note: null } }])
  })

  it('drops a change that goes back to the loaded value', () => {
    const edits = editCell(editCell(NO_EDITS, one, 'name', 'Bea', 'Ann'), one, 'name', 'Ann', 'Ann')
    expect(hasEdits(edits)).toBe(false)
  })

  it('does not change the original edits', () => {
    editCell(NO_EDITS, one, 'name', 'Bea', 'Ann')
    expect(NO_EDITS.updates.size).toBe(0)
  })
})

describe('deletes', () => {
  it('toggles and drops the row cell edits', () => {
    const edited = editCell(NO_EDITS, one, 'name', 'Bea', 'Ann')
    const deleted = toggleDelete(edited, one)
    expect(counts(deleted)).toEqual({ updated: 0, inserted: 0, deleted: 1 })
    expect(hasEdits(toggleDelete(deleted, one))).toBe(false)
  })
})

describe('new rows', () => {
  it('adds rows on top and keeps only the columns that were set', () => {
    let edits = addRow(addRow(NO_EDITS))
    expect(edits.inserts.map((r) => r.id)).toEqual([2, 1])
    edits = editNewCell(edits, 1, 'name', 'Cy')
    expect(edits.inserts[1].values).toEqual({ name: 'Cy' })
    expect(removeNewRow(edits, 2).inserts.map((r) => r.id)).toEqual([1])
  })

  it('can put a column of a new row back to its DEFAULT', () => {
    const edits = unsetNewCell(editNewCell(addRow(NO_EDITS), 1, 'name', 'Cy'), 1, 'name')
    expect(toChanges(edits)).toEqual([{ kind: 'insert', values: {} }])
  })
})

describe('toChanges', () => {
  it('runs deletes, then updates, then inserts oldest first', () => {
    let edits = addRow(NO_EDITS)
    edits = editNewCell(edits, 1, 'name', 'first')
    edits = addRow(edits)
    edits = editNewCell(edits, 2, 'name', 'second')
    edits = editCell(edits, { id: '2' }, 'name', 'x', 'y')
    edits = toggleDelete(edits, { id: '3' })
    expect(toChanges(edits).map((c) => c.kind)).toEqual(['delete', 'update', 'insert', 'insert'])
    expect(toChanges(edits).slice(2)).toEqual([
      { kind: 'insert', values: { name: 'first' } },
      { kind: 'insert', values: { name: 'second' } },
    ])
  })
})

describe('whole documents', () => {
  it('replaces a document until it is written back as loaded', () => {
    const edited = replaceDocument(NO_EDITS, one, '{"id": "1", "n": 2}', '{"id": "1", "n": 1}')
    expect(counts(edited)).toEqual({ updated: 1, inserted: 0, deleted: 0 })
    expect(toChanges(edited)).toEqual([{ kind: 'replace', key: one, document: '{"id": "1", "n": 2}' }])
    expect(hasEdits(replaceDocument(edited, one, '{"id": "1", "n": 1}', '{"id": "1", "n": 1}'))).toBe(false)
  })

  it('drops a replacement when the row is marked for deletion', () => {
    const edited = toggleDelete(replaceDocument(NO_EDITS, one, '{}', '{"id": "1"}'), one)
    expect(toChanges(edited)).toEqual([{ kind: 'delete', key: one }])
  })

  it('adds new documents, sent oldest first after everything else', () => {
    let edits = addDocument(NO_EDITS, '{}')
    edits = addDocument(edits, '{}')
    edits = editNewDocument(edits, 1, '{"first": true}')
    edits = replaceDocument(edits, one, '{"id": "1"}', '{"id": "1", "old": true}')
    expect(toChanges(edits).map((c) => c.kind)).toEqual(['replace', 'insertDocument', 'insertDocument'])
    expect(toChanges(edits)[1]).toEqual({ kind: 'insertDocument', document: '{"first": true}' })
    expect(counts(removeNewDocument(edits, 2))).toEqual({ updated: 1, inserted: 1, deleted: 0 })
  })
})

describe('cellAt', () => {
  const rows = [
    ['1', 'Ann', null],
    ['2', 'Bob', 'hi'],
  ]

  it('reads a loaded cell, with its pending edit', () => {
    const edits = editCell(NO_EDITS, one, 'name', 'Anna', 'Ann')
    expect(cellAt(edits, rows, columns, { row: 'row:0', column: 1 })).toEqual({
      value: 'Anna',
      unset: false,
      deleted: false,
    })
    expect(cellAt(edits, rows, columns, { row: 'row:1', column: 2 })?.value).toBe('hi')
  })

  it('says when the row is marked for deletion', () => {
    const edits = toggleDelete(NO_EDITS, one)
    expect(cellAt(edits, rows, columns, { row: 'row:0', column: 1 })?.deleted).toBe(true)
  })

  it('reads a new row, with columns left to their DEFAULT', () => {
    const edits = editNewCell(addRow(NO_EDITS), 1, 'name', 'Cy')
    expect(cellAt(edits, rows, columns, { row: 'new:1', column: 1 })).toEqual({
      value: 'Cy',
      unset: false,
      deleted: false,
    })
    expect(cellAt(edits, rows, columns, { row: 'new:1', column: 2 })).toEqual({
      value: null,
      unset: true,
      deleted: false,
    })
  })

  it('is null for a cell that is gone', () => {
    expect(cellAt(NO_EDITS, rows, columns, { row: 'row:5', column: 1 })).toBeNull()
    expect(cellAt(NO_EDITS, rows, columns, { row: 'new:9', column: 1 })).toBeNull()
    expect(cellAt(NO_EDITS, rows, columns, { row: 'row:0', column: 7 })).toBeNull()
  })
})

describe('DEFAULT', () => {
  it('is a pending change of a loaded row, read back as a DEFAULT cell', () => {
    const edits = editCell(NO_EDITS, one, 'note', DEFAULT, null)
    expect(toChanges(edits)).toEqual([{ kind: 'update', key: one, values: { note: DEFAULT } }])
    expect(cellAt(edits, [['1', 'Ann', null]], columns, { row: 'row:0', column: 2 })).toEqual({
      value: null,
      unset: true,
      deleted: false,
    })
  })

  it('goes over the wire as a marker, not as the text DEFAULT', () => {
    expect(JSON.stringify({ a: DEFAULT, b: 'DEFAULT' })).toBe('{"a":{"kind":"default"},"b":"DEFAULT"}')
  })

  it('adds a row holding another row’s values', () => {
    const edits = addRowWith(addRow(NO_EDITS), { name: 'Ann', email: null })
    expect(edits.inserts[0]).toEqual({ id: 2, values: { name: 'Ann', email: null } })
    expect(toChanges(edits)).toContainEqual({ kind: 'insert', values: { name: 'Ann', email: null } })
  })
})
