import { describe, expect, it } from 'vitest'
import { isNumericType, isUuidType, shortTypeName } from './columns'

describe('isNumericType', () => {
  it.each([
    'integer',
    'bigint',
    'numeric(10,2)',
    'double precision',
    'real',
    'int4',
    'float8',
    'int',
    'tinyint',
    'float',
    'smallmoney',
  ])('%s is numeric', (type) => expect(isNumericType(type)).toBe(true))
  it.each(['text', 'character varying(20)', 'uuid', 'text[]', 'jsonb', 'interval', 'nvarchar(40)', 'bit'])(
    '%s is not numeric',
    (type) => expect(isNumericType(type)).toBe(false),
  )
})

describe('shortTypeName', () => {
  it.each([
    ['character varying(80)', 'varchar(80)'],
    ['timestamp with time zone', 'timestamptz'],
    ['timestamp(6) with time zone', 'timestamptz(6)'],
    ['timestamp(3) without time zone', 'timestamp(3)'],
    ['double precision', 'float8'],
    ['character(2)', 'char(2)'],
    ['integer', 'integer'],
  ])('%s → %s', (type, short) => expect(shortTypeName(type)).toBe(short))
})

describe('isUuidType', () => {
  it('knows the native UUID types only', () => {
    expect(isUuidType('uuid')).toBe(true)
    expect(isUuidType('uniqueidentifier')).toBe(true)
    expect(isUuidType('UUID')).toBe(true)
    expect(isUuidType('char(36)')).toBe(false)
    expect(isUuidType('text')).toBe(false)
  })
})
