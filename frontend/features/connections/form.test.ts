import { describe, expect, it } from 'vitest'
import { canSubmit, emptyForm, formFromConnection, isValidPort, toInput, withEngine } from './form'

describe('withEngine', () => {
  it('moves an untouched default port to the new engine default', () => {
    expect(withEngine(emptyForm(), 'mysql').port).toBe('3306')
  })

  it('keeps a port the user changed', () => {
    expect(withEngine({ ...emptyForm(), port: '6543' }, 'mysql').port).toBe('6543')
  })
})

describe('isValidPort', () => {
  it.each(['1', '5432', '65535'])('accepts %s', (port) => expect(isValidPort(port)).toBe(true))
  it.each(['', '0', '65536', '54.3', 'abc'])('rejects %j', (port) => expect(isValidPort(port)).toBe(false))
})

describe('canSubmit', () => {
  it('needs a name and a valid port', () => {
    expect(canSubmit(emptyForm())).toBe(false)
    expect(canSubmit({ ...emptyForm(), name: 'local' })).toBe(true)
    expect(canSubmit({ ...emptyForm(), name: 'local', port: 'x' })).toBe(false)
  })

  it('needs a file, not a port, for SQLite', () => {
    const sqlite = { ...withEngine(emptyForm(), 'sqlite'), name: 'local' }
    expect(canSubmit(sqlite)).toBe(false)
    expect(canSubmit({ ...sqlite, database: '/data/app.db' })).toBe(true)
  })
})

describe('toInput', () => {
  it('converts the port and drops an empty password', () => {
    const input = toInput({ ...emptyForm(), name: 'local' }, 'abc')
    expect(input).toMatchObject({ id: 'abc', port: 5432 })
    expect(input).not.toHaveProperty('password', '')
    expect(input.password).toBeUndefined()
  })

  it('sends no server details for a SQLite file', () => {
    const form = {
      ...emptyForm(),
      engine: 'sqlite' as const,
      name: 'local',
      database: '/data/app.db',
      password: 'x',
    }
    expect(toInput(form)).toMatchObject({ host: '', port: 0, username: '', database: '/data/app.db' })
    expect(toInput(form).password).toBeUndefined()
  })

  it('keeps a typed password', () => {
    expect(toInput({ ...emptyForm(), password: 's3cret' }).password).toBe('s3cret')
  })

  it('round-trips a saved connection without its password', () => {
    const saved = {
      id: 'abc',
      name: 'prod',
      engine: 'postgres' as const,
      environment: 'production' as const,
      host: 'db',
      port: 5433,
      database: 'shop',
      username: 'app',
    }
    expect(toInput(formFromConnection(saved), saved.id)).toEqual({ ...saved, password: undefined })
  })
})
