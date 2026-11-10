import type { IconName } from '../../components/Icon'
import type { Engine, NewObjectKind } from '../../domain/connection'

/** What the backend creates; see `NewObject` in the Rust drivers. */
export type NewObject =
  | { kind: 'database'; name: string; collection?: string }
  | { kind: 'role'; name: string; database?: string }
  | { kind: 'user'; name: string; password: string; host?: string; database?: string }

/** A field of the New dialog beyond the name. */
export type ExtraField = 'password' | 'host' | 'collection' | 'database'

export type FieldValues = Record<'name' | ExtraField, string>

export const EMPTY_VALUES: FieldValues = { name: '', password: '', host: '', collection: '', database: '' }

export const NOUNS: Record<NewObjectKind, string> = { database: 'database', role: 'role', user: 'user' }

/** The connection menu's items in its New section. */
export const MENU_LABELS: Record<NewObjectKind, string> = {
  database: 'Database…',
  role: 'Role…',
  user: 'User…',
}

export const MENU_ICONS: Record<NewObjectKind, IconName> = {
  database: 'database',
  role: 'role',
  user: 'user',
}

/** The fields a kind needs on an engine, in form order. */
export function fieldsOf(engine: Engine, kind: NewObjectKind): ExtraField[] {
  const mongo = engine === 'mongodb'
  if (kind === 'database') return mongo ? ['collection'] : []
  if (kind === 'role') return mongo ? ['database'] : []
  if (mongo) return ['password', 'database']
  return engine === 'mysql' || engine === 'mariadb' ? ['password', 'host'] : ['password']
}

/** Fields the form leaves optional, with what an empty one means. */
export const PLACEHOLDERS: Partial<Record<ExtraField, string>> = { host: '%', database: 'admin' }

/** What creating `kind` on `engine` does, in one line under the form. */
export function noteOf(engine: Engine, kind: NewObjectKind): string {
  if (kind === 'database') {
    return engine === 'mongodb'
      ? 'MongoDB keeps a database once it holds a collection, so it is created with its first one.'
      : 'An empty database, owned by this connection’s user.'
  }
  if (kind === 'role') {
    return engine === 'sqlserver'
      ? 'A server role, without permissions. Grant them in the console.'
      : 'A role without privileges. Grant them in the console.'
  }
  switch (engine) {
    case 'postgres':
      return 'A role that can sign in, without privileges. Grant them in the console.'
    case 'sqlserver':
      return 'A login with SQL Server authentication, without permissions. Grant them in the console.'
    case 'redis':
      return 'An ACL user that is on, without permissions. Grant them with ACL SETUSER in the console.'
    default:
      return 'A user without privileges. Grant them in the console.'
  }
}

/** Every required field is filled: the name, a user's password, MongoDB's first collection. */
export function isComplete(values: FieldValues, fields: ExtraField[]): boolean {
  const required: ('name' | ExtraField)[] = [
    'name',
    ...fields.filter((f) => f !== 'host' && f !== 'database'),
  ]
  return required.every((field) =>
    field === 'password' ? values[field] !== '' : values[field].trim() !== '',
  )
}

/** The request for `kind` from the form's values. Names are trimmed; passwords never are. */
export function toNewObject(kind: NewObjectKind, fields: ExtraField[], values: FieldValues): NewObject {
  const name = values.name.trim()
  const optional = (field: ExtraField) => (fields.includes(field) ? values[field].trim() : undefined)
  switch (kind) {
    case 'database':
      return { kind, name, collection: optional('collection') }
    case 'role':
      return { kind, name, database: optional('database') }
    case 'user':
      return { kind, name, password: values.password, host: optional('host'), database: optional('database') }
  }
}
