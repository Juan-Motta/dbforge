import { describe, expect, it } from 'vitest'
import { drawnRows, drawsWhole, rowWindow, windowReach } from './useWindowedRows'

describe('rowWindow', () => {
  it('takes the rows in sight and some around them, and stands in for the rest', () => {
    expect(rowWindow(0, 100, 20, 1000, 2)).toEqual({ start: 0, end: 7, before: 0, after: 19_860 })
    expect(rowWindow(2010, 100, 20, 1000, 2)).toEqual({ start: 98, end: 108, before: 1960, after: 17_840 })
  })

  it('stays inside the list at its end, when it is short and when it is empty', () => {
    expect(rowWindow(19_900, 100, 20, 1000, 2)).toEqual({ start: 993, end: 1000, before: 19_860, after: 0 })
    expect(rowWindow(0, 400, 20, 3, 2)).toEqual({ start: 0, end: 3, before: 0, after: 0 })
    // Scrolled further than a list that just got shorter.
    expect(rowWindow(5000, 100, 20, 3, 2)).toEqual({ start: 3, end: 3, before: 60, after: 0 })
    expect(rowWindow(0, 0, 20, 0, 2)).toEqual({ start: 0, end: 0, before: 0, after: 0 })
  })
})

describe('drawnRows', () => {
  it('draws the window with a gap for the rows before and after it', () => {
    expect(drawnRows(2, 4, null, 10)).toEqual([
      { gap: 2, from: 0 },
      { index: 2 },
      { index: 3 },
      { gap: 6, from: 4 },
    ])
    expect(drawnRows(0, 2, null, 2)).toEqual([{ index: 0 }, { index: 1 }])
    expect(drawnRows(0, 0, null, 0)).toEqual([])
  })

  it('keeps a row out of the window drawn, where it belongs', () => {
    expect(drawnRows(5, 6, 1, 8)).toEqual([
      { gap: 1, from: 0 },
      { index: 1 },
      { gap: 3, from: 2 },
      { index: 5 },
      { gap: 2, from: 6 },
    ])
    expect(drawnRows(0, 1, 7, 8)).toEqual([{ index: 0 }, { gap: 6, from: 1 }, { index: 7 }])
    // In the window already, or not a row: nothing more to draw.
    expect(drawnRows(0, 2, 1, 2)).toEqual([{ index: 0 }, { index: 1 }])
    expect(drawnRows(0, 1, -1, 1)).toEqual([{ index: 0 }])
  })
})

describe('windowReach', () => {
  it('draws two views past each side and moves every half view', () => {
    // 520 px of 26 px rows: 20 in sight.
    expect(windowReach(520, 26)).toEqual({ overscan: 40, step: 10 })
  })

  it('keeps a floor of rows before the list is measured', () => {
    expect(windowReach(0, 26)).toEqual({ overscan: 12, step: 1 })
  })
})

describe('drawsWhole', () => {
  it('draws a page of the largest size whole, so a fast scroll never outruns it', () => {
    // 1000 rows of 24 columns, with the row number.
    expect(drawsWhole(1000, 25)).toBe(true)
  })

  it('draws only the rows in sight past the size that would cost more than a gap', () => {
    expect(drawsWhole(1000, 26)).toBe(false)
    expect(drawsWhole(30_000, 1)).toBe(false)
  })
})
