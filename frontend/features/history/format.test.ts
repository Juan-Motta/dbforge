import { describe, expect, it } from 'vitest'
import type { HistoryEntry } from './api'
import { firstLine, formatDuration, groupByDay, outcomeLabel } from './format'
import { messages } from './messages'

const entry = (overrides: Partial<HistoryEntry> = {}): HistoryEntry => ({
  id: '1',
  executedAt: 0,
  connectionId: 'c1',
  connectionName: 'local',
  database: null,
  sql: 'SELECT 1',
  elapsedMs: 1,
  statements: 1,
  rows: 0,
  error: null,
  ...overrides,
})

describe('groupByDay', () => {
  const now = new Date(2026, 10, 6, 15, 0)
  const at = (day: number, hour: number) => new Date(2026, 10, day, hour).getTime()

  it('labels today, yesterday and older days, keeping the order', () => {
    const groups = groupByDay(
      [
        entry({ id: 'a', executedAt: at(6, 9) }),
        entry({ id: 'b', executedAt: at(6, 0) }),
        entry({ id: 'c', executedAt: at(5, 23) }),
        entry({ id: 'd', executedAt: at(2, 12) }),
      ],
      now,
      messages.en,
    )
    expect(groups.map((g) => [g.label, g.entries.map((e) => e.id)])).toEqual([
      ['Today', ['a', 'b']],
      ['Yesterday', ['c']],
      ['Mon, Nov 2', ['d']],
    ])
  })

  it('labels the days in Spanish', () => {
    const groups = groupByDay(
      [entry({ executedAt: at(6, 9) }), entry({ executedAt: at(5, 9) })],
      now,
      messages.es,
    )
    expect(groups.map((g) => g.label)).toEqual(['Hoy', 'Ayer'])
  })
})

describe('formatDuration', () => {
  it.each([
    [38, '38 ms'],
    [8200, '8.2 s'],
    [150_000, '2.5 min'],
  ])('%d ms → %s', (ms, text) => expect(formatDuration(ms)).toBe(text))
})

describe('outcomeLabel', () => {
  it('names errors, scripts and row counts', () => {
    expect(outcomeLabel(entry({ error: 'boom' }), messages.en)).toBe('error')
    expect(outcomeLabel(entry({ statements: 3 }), messages.en)).toBe('3 statements')
    expect(outcomeLabel(entry({ rows: 1 }), messages.en)).toBe('1 row')
    expect(outcomeLabel(entry({ rows: 18204 }), messages.en)).toBe('18,204 rows')
    expect(outcomeLabel(entry({ rows: 1 }), messages.es)).toBe('1 fila')
  })
})

describe('firstLine', () => {
  it('skips blank lines and indentation', () => {
    expect(firstLine('\n   SELECT id\n  FROM t')).toBe('SELECT id')
  })
})
