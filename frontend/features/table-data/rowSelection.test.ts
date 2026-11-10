import { describe, expect, it } from 'vitest'
import { modeOf, NO_SELECTION, rowsAsTsv, selectRows } from './rowSelection'

const order = ['new:1', 'a', 'b', 'c', 'd']
const ids = (selection: { selected: Set<string> }) => [...selection.selected]

describe('selectRows', () => {
  it('picks one row, and clears it when clicked again', () => {
    const one = selectRows(NO_SELECTION, 'b', 'single', order)
    expect(ids(one)).toEqual(['b'])
    expect(ids(selectRows(one, 'b', 'single', order))).toEqual([])
    expect(ids(selectRows(one, 'c', 'single', order))).toEqual(['c'])
  })

  it('adds and removes rows one by one', () => {
    let selection = selectRows(NO_SELECTION, 'a', 'single', order)
    selection = selectRows(selection, 'c', 'toggle', order)
    expect(ids(selection)).toEqual(['a', 'c'])
    selection = selectRows(selection, 'a', 'toggle', order)
    expect(ids(selection)).toEqual(['c'])
  })

  it('selects a block from the anchor, either way, as a drag does', () => {
    const start = selectRows(NO_SELECTION, 'b', 'single', order)
    expect(ids(selectRows(start, 'd', 'range', order))).toEqual(['b', 'c', 'd'])
    const back = selectRows(selectRows(start, 'd', 'range', order), 'new:1', 'range', order)
    expect(ids(back)).toEqual(['new:1', 'a', 'b'])
    expect(back.anchor).toBe('b')
    expect(ids(selectRows(NO_SELECTION, 'c', 'range', order))).toEqual(['c'])
  })

  it('reads the mode from the modifier keys', () => {
    expect(modeOf({ shiftKey: true, metaKey: true, ctrlKey: false })).toBe('range')
    expect(modeOf({ shiftKey: false, metaKey: true, ctrlKey: false })).toBe('toggle')
    expect(modeOf({ shiftKey: false, metaKey: false, ctrlKey: true })).toBe('toggle')
    expect(modeOf({ shiftKey: false, metaKey: false, ctrlKey: false })).toBe('single')
  })
})

describe('rowsAsTsv', () => {
  it('writes rows for a spreadsheet, quoting what would break the columns', () => {
    expect(
      rowsAsTsv([
        ['1', 'Ann', null],
        ['2', 'a\tb', 'say "hi"\nthen'],
      ]),
    ).toBe('1\tAnn\tNULL\n2\t"a\tb"\t"say ""hi""\nthen"')
  })
})
