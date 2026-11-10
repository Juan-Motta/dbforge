import { describe, expect, it } from 'vitest'
import { formatTemporal, parseTemporal, temporalKindOf, temporalOf, withParts } from './temporal'

describe('temporalKindOf', () => {
  it('knows each engine’s date and time types', () => {
    expect(temporalKindOf('date')).toBe('date')
    expect(temporalKindOf('time without time zone')).toBe('time')
    expect(temporalKindOf('time(6)')).toBe('time')
    expect(temporalKindOf('timestamp(3) with time zone')).toBe('timestamptz')
    expect(temporalKindOf('timestamp without time zone')).toBe('timestamp')
    expect(temporalKindOf('datetime(6)')).toBe('timestamp')
    expect(temporalKindOf('datetime2(7)')).toBe('timestamp')
    expect(temporalKindOf('datetimeoffset(7)')).toBe('timestamptz')
    expect(temporalKindOf('text')).toBeNull()
    expect(temporalKindOf('time with time zone')).toBeNull()
  })
})

describe('parseTemporal and formatTemporal', () => {
  it('read each engine’s text and write it back the same way', () => {
    const pg = '2026-09-16 14:00:27.123456+05:30'
    expect(formatTemporal(parseTemporal(pg, 'timestamptz')!, 'timestamptz')).toBe(pg)
    expect(parseTemporal('2026-11-05 10:00:00+00', 'timestamptz')?.offset).toBe(0)
    expect(parseTemporal('2026-11-05 10:00:00-03', 'timestamptz')?.offset).toBe(-180)
    const mssql = '2026-11-05 10:00:00.1234567 +00:00'
    expect(formatTemporal(parseTemporal(mssql, 'timestamptz')!, 'timestamptz', true)).toBe(mssql)
    expect(formatTemporal(parseTemporal('2026-02-03T04:05', 'timestamp')!, 'timestamp')).toBe(
      '2026-02-03 04:05:00',
    )
    expect(formatTemporal(parseTemporal('2026-02-03', 'date')!, 'date')).toBe('2026-02-03')
    expect(formatTemporal(parseTemporal('09:30:00.5', 'time')!, 'time')).toBe('09:30:00.5')
  })

  it('refuses what isn’t a real date or time', () => {
    expect(parseTemporal('2026-02-30', 'date')).toBeNull()
    expect(parseTemporal('2028-02-29', 'date')).not.toBeNull()
    expect(parseTemporal('24:00:00', 'time')).toBeNull()
    expect(parseTemporal('2026-01-01 10:00:00', 'timestamptz')).toBeNull()
    expect(parseTemporal('yesterday', 'timestamp')).toBeNull()
  })
})

describe('editing parts', () => {
  it('pulls the day back into a shorter month', () => {
    const march31 = parseTemporal('2026-03-31', 'date')!
    expect(withParts(march31, { month: 2 }).day).toBe(28)
    expect(withParts(march31, { month: 2, year: 2028 }).day).toBe(29)
  })

  it('takes now from this computer’s clock and offset', () => {
    const now = temporalOf(new Date(2026, 8, 16, 14, 0, 27))
    expect(formatTemporal(now, 'timestamp')).toBe('2026-09-16 14:00:27')
    expect(now.offset).toBe(-new Date(2026, 8, 16).getTimezoneOffset())
  })
})
