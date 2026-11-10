import { describe, expect, it } from 'vitest'
import { rowWindow } from './useWindowedRows'

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
