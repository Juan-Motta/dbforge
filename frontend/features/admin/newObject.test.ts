import { describe, expect, it } from 'vitest'
import { messages } from './messages'
import { EMPTY_VALUES, fieldsOf, isComplete, noteOf, titleOf, toNewObject } from './newObject'

describe('fieldsOf', () => {
  it('asks each engine only for what it needs', () => {
    expect(fieldsOf('postgres', 'database')).toEqual([])
    expect(fieldsOf('postgres', 'user')).toEqual(['password'])
    expect(fieldsOf('mysql', 'user')).toEqual(['password', 'host'])
    expect(fieldsOf('mariadb', 'user')).toEqual(['password', 'host'])
    expect(fieldsOf('mongodb', 'database')).toEqual(['collection'])
    expect(fieldsOf('mongodb', 'role')).toEqual(['database'])
    expect(fieldsOf('mongodb', 'user')).toEqual(['password', 'database'])
    expect(fieldsOf('redis', 'user')).toEqual(['password'])
  })
})

describe('isComplete', () => {
  it('needs a name, a user’s password and MongoDB’s first collection, not the optional fields', () => {
    const user = fieldsOf('mysql', 'user')
    expect(isComplete({ ...EMPTY_VALUES, name: ' ' }, [])).toBe(false)
    expect(isComplete({ ...EMPTY_VALUES, name: 'shop' }, [])).toBe(true)
    expect(isComplete({ ...EMPTY_VALUES, name: 'ann' }, user)).toBe(false)
    expect(isComplete({ ...EMPTY_VALUES, name: 'ann', password: ' ' }, user)).toBe(true)
    expect(isComplete({ ...EMPTY_VALUES, name: 'shop' }, fieldsOf('mongodb', 'database'))).toBe(false)
  })
})

describe('toNewObject', () => {
  it('trims names, keeps passwords as typed and sends only the fields the engine uses', () => {
    const values = { name: ' ann ', password: ' p ', host: ' localhost ', collection: 'x', database: ' app ' }
    expect(toNewObject('user', fieldsOf('mysql', 'user'), values)).toEqual({
      kind: 'user',
      name: 'ann',
      password: ' p ',
      host: 'localhost',
      database: undefined,
    })
    expect(toNewObject('role', fieldsOf('mongodb', 'role'), values)).toEqual({
      kind: 'role',
      name: 'ann',
      database: 'app',
    })
    expect(toNewObject('database', fieldsOf('postgres', 'database'), values)).toEqual({
      kind: 'database',
      name: 'ann',
      collection: undefined,
    })
  })
})

describe('texts', () => {
  it('writes the dialog in the interface language', () => {
    expect(titleOf('user', 'local', messages.en)).toBe('New user on local')
    expect(titleOf('database', 'local', messages.es)).toBe('Nueva base de datos en local')
    expect(noteOf('postgres', 'role', messages.en)).toBe(
      'A role without privileges. Grant them in the console.',
    )
    expect(noteOf('postgres', 'role', messages.es)).toBe('Un rol sin privilegios. Otórgalos en la consola.')
  })
})
