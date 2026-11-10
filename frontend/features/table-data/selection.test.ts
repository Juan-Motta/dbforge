import { describe, expect, it } from 'vitest'
import { cellKey, cellsAsTsv, modeOf, NO_SELECTION, select, selectedCells, selectedRows } from './selection'

const order = ['new:1', 'a', 'b', 'c']
const at = (row: string, column: number) => ({ row, column })
const keys = (selection: { cells: Set<string> }) =>
  selectedCells({ ...NO_SELECTION, cells: selection.cells }, order).map((c) => `${c.row}${c.column}`)

describe('select', () => {
  it('picks one cell, adds and removes cells, and selects whole rows from their number', () => {
    let selection = select(NO_SELECTION, at('a', 1), 'single', order, 3)
    expect(keys(selection)).toEqual(['a1'])
    selection = select(selection, at('c', 2), 'toggle', order, 3)
    expect(keys(selection)).toEqual(['a1', 'c2'])
    selection = select(selection, at('a', 1), 'toggle', order, 3)
    expect(keys(selection)).toEqual(['c2'])
    expect(keys(select(NO_SELECTION, at('b', 0), 'single', order, 3, true))).toEqual(['b0', 'b1', 'b2'])
  })

  it('selects a block from the anchor, either way, as a drag does', () => {
    const start = select(NO_SELECTION, at('b', 2), 'start', order, 3)
    expect(keys(select(start, at('new:1', 0), 'range', order, 3))).toEqual([
      'new:10',
      'new:11',
      'new:12',
      'a0',
      'a1',
      'a2',
      'b0',
      'b1',
      'b2',
    ])
    const rows = select(
      select(NO_SELECTION, at('a', 0), 'single', order, 2, true),
      at('c', 0),
      'range',
      order,
      2,
      true,
    )
    expect(keys(rows)).toEqual(['a0', 'a1', 'b0', 'b1', 'c0', 'c1'])
    expect(selectedRows(rows, order)).toEqual(['a', 'b', 'c'])
  })

  it('reads the mode from the modifier keys', () => {
    expect(modeOf({ shiftKey: true, metaKey: true, ctrlKey: false })).toBe('range')
    expect(modeOf({ shiftKey: false, metaKey: true, ctrlKey: false })).toBe('toggle')
    expect(modeOf({ shiftKey: false, metaKey: false, ctrlKey: true })).toBe('toggle')
    expect(modeOf({ shiftKey: false, metaKey: false, ctrlKey: false })).toBe('single')
  })
})

describe('cellsAsTsv', () => {
  it('writes the selected block for a spreadsheet, quoting what would break it', () => {
    const values: Record<string, string | null> = {
      [cellKey(at('a', 0))]: '1',
      [cellKey(at('a', 1))]: 'a\tb',
      [cellKey(at('c', 1))]: null,
      [cellKey(at('c', 2))]: 'say "hi"',
    }
    const selection = { cells: new Set(Object.keys(values)), anchor: null }
    expect(cellsAsTsv(selection, order, (cell) => values[cellKey(cell)])).toBe(
      '1\t"a\tb"\t\n\tNULL\t"say ""hi"""',
    )
  })
})
