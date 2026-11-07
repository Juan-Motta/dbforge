import type { HistoryEntry } from './api'

const DAY_MS = 24 * 60 * 60 * 1000

/** Splits newest-first entries into Today, Yesterday and dated groups, in local time. */
export function groupByDay(entries: HistoryEntry[], now: Date): { label: string; entries: HistoryEntry[] }[] {
  const today = startOfDay(now)
  const groups: { label: string; entries: HistoryEntry[] }[] = []
  for (const entry of entries) {
    const label = dayLabel(startOfDay(new Date(entry.executedAt)), today)
    const last = groups.at(-1)
    if (last?.label === label) last.entries.push(entry)
    else groups.push({ label, entries: [entry] })
  }
  return groups
}

function dayLabel(day: number, today: number): string {
  // Rounding absorbs the hour that daylight saving time adds or removes.
  const daysAgo = Math.round((today - day) / DAY_MS)
  if (daysAgo === 0) return 'Today'
  if (daysAgo === 1) return 'Yesterday'
  return new Date(day).toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' })
}

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()
}

/** 14:05 */
export function formatTime(executedAt: number): string {
  return new Date(executedAt).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })
}

/** 38 ms, 8.2 s, 2.5 min */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`
  return `${(ms / 60_000).toFixed(1)} min`
}

/** What a run produced: "14 rows", "3 statements", or "error". */
export function outcomeLabel(entry: HistoryEntry): string {
  if (entry.error !== null) return 'error'
  if (entry.statements > 1) return `${entry.statements} statements`
  return entry.rows === 1 ? '1 row' : `${entry.rows.toLocaleString('en-US')} rows`
}

/** The first non-empty line, so a multi-line query reads well in one row. */
export function firstLine(sql: string): string {
  return (
    sql
      .split('\n')
      .map((line) => line.trim())
      .find(Boolean) ?? ''
  )
}

export type HistoryScope = 'connection' | 'all'

/** The runs of one connection, or all of them. */
export function inScope<T extends { connectionId: string }>(
  entries: T[],
  scope: HistoryScope,
  connectionId: string,
): T[] {
  return scope === 'all' ? entries : entries.filter((entry) => entry.connectionId === connectionId)
}
