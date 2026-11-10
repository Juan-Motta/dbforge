import type { IconName } from '../../components/Icon'
import type { Engine, NewObjectKind } from '../../domain/connection'
import type { AdminMessages } from './messages'

/** What the backend creates; see `NewObject` in the Rust drivers. */
export type NewObject =
  | { kind: 'database'; name: string; collection?: string }
  | { kind: 'role'; name: string; database?: string }
  | { kind: 'user'; name: string; password: string; host?: string; database?: string }

/** A field of the New dialog beyond the name. */
export type ExtraField = 'password' | 'host' | 'collection' | 'database'

export type FieldValues = Record<'name' | ExtraField, string>

export const EMPTY_VALUES: FieldValues = { name: '', password: '', host: '', collection: '', database: '' }

/** The connection menu's item for `kind` in its New section. */
export function menuLabelOf(kind: NewObjectKind, m: AdminMessages): string {
  return { database: m.menuDatabase, role: m.menuRole, user: m.menuUser }[kind]
}

/** The New dialog's title for `kind` on a connection. */
export function titleOf(kind: NewObjectKind, connection: string, m: AdminMessages): string {
  return { database: m.titleDatabase, role: m.titleRole, user: m.titleUser }[kind](connection)
}

/** The New dialog's submit button for `kind`. */
export function createLabelOf(kind: NewObjectKind, m: AdminMessages): string {
  return { database: m.createDatabase, role: m.createRole, user: m.createUser }[kind]
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
export function noteOf(engine: Engine, kind: NewObjectKind, m: AdminMessages): string {
  if (kind === 'database') return engine === 'mongodb' ? m.noteMongoDatabase : m.noteDatabase
  if (kind === 'role') return engine === 'sqlserver' ? m.noteServerRole : m.noteRole
  switch (engine) {
    case 'postgres':
      return m.notePostgresUser
    case 'sqlserver':
      return m.noteSqlServerUser
    case 'redis':
      return m.noteRedisUser
    default:
      return m.noteUser
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
