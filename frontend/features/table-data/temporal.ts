/** The kinds of date and time a column can hold, as the editor offers them. */
export type TemporalKind = 'date' | 'time' | 'timestamp' | 'timestamptz'

/** A date and time split into the parts the editor's controls show. */
export interface Temporal {
  year: number
  /** 1–12. */
  month: number
  day: number
  hour: number
  minute: number
  second: number
  /** Digits after the decimal point, kept as typed (`''` for none). */
  fraction: string
  /** Minutes east of UTC; only `timestamptz` has one. */
  offset: number | null
}

/**
 * The kind of a column from its type as the server names it: PostgreSQL's
 * `timestamp(3) with time zone`, MySQL's `datetime(6)`, SQL Server's
 * `datetimeoffset(7)`… `null` for anything else (SQLite keeps dates as text).
 */
export function temporalKindOf(dataType: string): TemporalKind | null {
  const type = dataType
    .toLowerCase()
    .replace(/\(\d+\)/g, '')
    .trim()
  if (type === 'date') return 'date'
  if (type === 'time' || type === 'time without time zone') return 'time'
  if (type === 'timestamptz' || type === 'timestamp with time zone' || type === 'datetimeoffset')
    return 'timestamptz'
  if (['timestamp', 'timestamp without time zone', 'datetime', 'datetime2', 'smalldatetime'].includes(type)) {
    return 'timestamp'
  }
  return null
}

const DATE = '(\\d{4})-(\\d{2})-(\\d{2})'
const TIME = '(\\d{2}):(\\d{2})(?::(\\d{2})(?:\\.(\\d+))?)?'
const OFFSET = '\\s?(Z|[+-]\\d{2}(?::?\\d{2})?)'
const PATTERNS: Record<TemporalKind, RegExp> = {
  date: new RegExp(`^${DATE}$`),
  time: new RegExp(`^${TIME}$`),
  timestamp: new RegExp(`^${DATE}[ T]${TIME}$`),
  timestamptz: new RegExp(`^${DATE}[ T]${TIME}${OFFSET}$`),
}

export function daysInMonth(year: number, month: number): number {
  return new Date(Date.UTC(year, month, 0)).getUTCDate()
}

/** The parts of `text` read as `kind`; `null` when it isn't a valid value of that kind. */
export function parseTemporal(text: string, kind: TemporalKind): Temporal | null {
  const match = PATTERNS[kind].exec(text.trim())
  if (!match) return null
  const parts = match.slice(1)
  const hasDate = kind !== 'time'
  const [year, month, day] = hasDate ? parts.splice(0, 3).map(Number) : [2000, 1, 1]
  const hasTime = kind !== 'date'
  const [hour, minute, second, fraction] = hasTime ? parts.splice(0, 4) : ['0', '0', '0', '']
  const value: Temporal = {
    year,
    month,
    day,
    hour: Number(hour),
    minute: Number(minute),
    second: Number(second ?? 0),
    fraction: fraction ?? '',
    offset: kind === 'timestamptz' ? offsetMinutes(parts[0]) : null,
  }
  return isValid(value) ? value : null
}

function offsetMinutes(text: string): number {
  if (text === 'Z') return 0
  const sign = text.startsWith('-') ? -1 : 1
  const digits = text.slice(1).replace(':', '')
  return sign * (Number(digits.slice(0, 2)) * 60 + Number(digits.slice(2) || 0))
}

function isValid({ year, month, day, hour, minute, second, offset }: Temporal): boolean {
  return (
    year >= 1 &&
    month >= 1 &&
    month <= 12 &&
    day >= 1 &&
    day <= daysInMonth(year, month) &&
    hour < 24 &&
    minute < 60 &&
    second < 60 &&
    (offset === null || Math.abs(offset) < 24 * 60)
  )
}

const pad = (value: number, width = 2) => String(value).padStart(width, '0')

/**
 * The value as the server reads it back: `YYYY-MM-DD`, `HH:MM:SS[.f]`, or both
 * with a space between, plus `+HH:MM` for `timestamptz` (after a space when
 * `offsetSpace`, as SQL Server writes it).
 */
export function formatTemporal(value: Temporal, kind: TemporalKind, offsetSpace = false): string {
  const date = `${pad(value.year, 4)}-${pad(value.month)}-${pad(value.day)}`
  const time = `${pad(value.hour)}:${pad(value.minute)}:${pad(value.second)}${value.fraction ? `.${value.fraction}` : ''}`
  switch (kind) {
    case 'date':
      return date
    case 'time':
      return time
    case 'timestamp':
      return `${date} ${time}`
    case 'timestamptz': {
      const offset = value.offset ?? 0
      const sign = offset < 0 ? '-' : '+'
      const zone = `${sign}${pad(Math.floor(Math.abs(offset) / 60))}:${pad(Math.abs(offset) % 60)}`
      return `${date} ${time}${offsetSpace ? ' ' : ''}${zone}`
    }
  }
}

/** `date` as this computer's local time, with its offset. */
export function temporalOf(date: Date): Temporal {
  return {
    year: date.getFullYear(),
    month: date.getMonth() + 1,
    day: date.getDate(),
    hour: date.getHours(),
    minute: date.getMinutes(),
    second: date.getSeconds(),
    fraction: '',
    offset: -date.getTimezoneOffset(),
  }
}

/** `value` with `changes`, the day pulled back into a shorter month (31 March → February keeps the 28th or 29th). */
export function withParts(value: Temporal, changes: Partial<Temporal>): Temporal {
  const next = { ...value, ...changes }
  return { ...next, day: Math.min(next.day, daysInMonth(next.year, next.month)) }
}
