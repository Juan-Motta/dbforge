import { describe, expect, it } from 'vitest'
import {
  addRow,
  counts,
  editCell,
  editNewCell,
  hasEdits,
  keyOf,
  NO_EDITS,
  removeNewRow,
  rowKey,
  toChanges,
  toggleDelete,
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
