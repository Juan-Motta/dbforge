import { DEFAULT_PORTS, type Connection, type Engine, type Environment } from '../../domain/connection'
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

export function canSubmit(form: ConnectionForm): boolean {
  return form.name.trim() !== '' && isValidPort(form.port)
}

/** An empty password means "no change", so it is left out. */
export function toInput(form: ConnectionForm, id?: string): ConnectionInput {
  const { password, port, ...rest } = form
  return { ...rest, id, port: Number(port), password: password === '' ? undefined : password }
}
