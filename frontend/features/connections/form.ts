import {
  DEFAULT_PORTS,
  isFileEngine,
  type Connection,
  type Engine,
  type Environment,
} from '../../domain/connection'
import type { ConnectionInput } from './api'

/** Form values as typed by the user. */
export interface ConnectionForm {
  name: string
  engine: Engine
  environment: Environment | null
  host: string
  port: string
  database: string
  username: string
  password: string
}

export function emptyForm(): ConnectionForm {
  return {
    name: '',
    engine: 'postgres',
    environment: null,
    host: 'localhost',
    port: String(DEFAULT_PORTS.postgres),
    database: '',
    username: '',
    password: '',
  }
}

export function formFromConnection(connection: Connection): ConnectionForm {
  return { ...connection, port: String(connection.port), password: '' }
}

/** Switches engine, moving the port to the new default unless the user changed it. */
export function withEngine(form: ConnectionForm, engine: Engine): ConnectionForm {
  const usesDefaultPort = form.port === String(DEFAULT_PORTS[form.engine])
  return { ...form, engine, port: usesDefaultPort ? String(DEFAULT_PORTS[engine]) : form.port }
}

export function isValidPort(port: string): boolean {
  const value = Number(port)
  return Number.isInteger(value) && value > 0 && value <= 65535
}

/** Enough to try connecting: a file for SQLite, a valid port for servers. */
export function canTest(form: ConnectionForm): boolean {
  return isFileEngine(form.engine) ? form.database.trim() !== '' : isValidPort(form.port)
}

export function canSubmit(form: ConnectionForm): boolean {
  return form.name.trim() !== '' && canTest(form)
}

/** An empty password means "no change", so it is left out. A file needs no server details. */
export function toInput(form: ConnectionForm, id?: string): ConnectionInput {
  const { password, port, ...rest } = form
  if (isFileEngine(form.engine)) {
    return { ...rest, id, host: '', port: 0, username: '', password: undefined }
  }
  return { ...rest, id, port: Number(port), password: password === '' ? undefined : password }
}
