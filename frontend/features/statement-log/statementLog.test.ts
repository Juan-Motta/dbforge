import { describe, expect, it } from 'vitest'
import type { StatementEntry } from './api'
import { clockTime, duration, merge, runnable, script, visible } from './statementLog'

function entry(
  id: number,
  text = `SELECT ${id}`,
  origin: StatementEntry['origin'] = 'console',
): StatementEntry {
  return {
    id,
    at: 0,
    connectionId: 'pg',
    database: 'shop',
    origin,
    text,
    elapsedMs: 1,
    rows: 1,
    error: null,
  }
}

describe('merge', () => {
  it('adds new entries by id, in order, without the ones already there', () => {
    const merged = merge([entry(1), entry(3)], [entry(3), entry(2), entry(4)])
    expect(merged.map((e) => e.id)).toEqual([1, 2, 3, 4])
  })

  it('keeps the newest up to the capacity, and the same list when nothing is new', () => {
    const entries = [entry(1), entry(2)]
    expect(merge(entries, [entry(2)])).toBe(entries)
    expect(merge(entries, [entry(3)], 2).map((e) => e.id)).toEqual([2, 3])
  })
})

describe('visible', () => {
  it('filters by what the statements were for', () => {
    const entries = [
      entry(1, 'a', 'console'),
      entry(2, 'b', 'edit'),
      entry(3, 'c', 'metadata'),
      entry(4, 'd', 'browse'),
    ]
    expect(visible(entries, 'all')).toHaveLength(4)
    expect(visible(entries, 'changes').map((e) => e.id)).toEqual([2])
    expect(visible(entries, 'reads').map((e) => e.id)).toEqual([4])
    expect(visible(entries, 'metadata').map((e) => e.id)).toEqual([3])
  })
})

describe('runnable and script', () => {
  it('ends SQL with a semicolon, leaves other languages as they are', () => {
    expect(runnable(entry(1, 'SELECT 1 '), 'postgres')).toBe('SELECT 1;')
    expect(runnable(entry(1, 'COMMIT;'), 'sqlserver')).toBe('COMMIT;')
    expect(runnable(entry(1, 'GET "k"'), 'redis')).toBe('GET "k"')
    expect(runnable(entry(1, 'db.getCollection("a").find({})'), 'mongodb')).toBe(
      'db.getCollection("a").find({})',
    )
  })

  it('joins statements in order, each runnable on its engine', () => {
    const entries = [entry(1, 'BEGIN'), { ...entry(2, 'GET "k"'), connectionId: 'r' }]
    expect(script(entries, (id) => (id === 'r' ? 'redis' : 'postgres'))).toBe('BEGIN;\n\nGET "k"')
  })
})

describe('formatting', () => {
  it('writes the time of day with milliseconds and short durations', () => {
    const at = new Date(2026, 10, 10, 9, 5, 7, 42).getTime()
    expect(clockTime(at)).toBe('09:05:07.042')
    expect(duration(8)).toBe('8 ms')
    expect(duration(1240)).toBe('1.24 s')
  })
})
