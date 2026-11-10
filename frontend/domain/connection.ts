import { messages, type SharedMessages } from '../lib/messages'

export type Engine =
  'postgres' | 'mysql' | 'mariadb' | 'sqlite' | 'sqlserver' | 'mongodb' | 'redis' | 'cosmosdb'

/** A saved connection, as returned by the backend. The password is never included. */
export interface Connection {
  id: string
  name: string
  engine: Engine
  /** The id of the connection's tag (see `features/tags`), or none. */
  environment: string | null
  host: string
  port: number
  database: string
  username: string
  /** Encrypts the connection (Redis, MongoDB, and Cosmos DB unless it is the local emulator). */
  tls: boolean
  /** Accepts a certificate no authority signed, as development servers have. */
  allowSelfSigned: boolean
  /** MongoDB: the host names a DNS seed list (`mongodb+srv://`). */
  srv: boolean
  /** What AI agents connected over MCP may do with it. */
  agentAccess: AgentAccess
}

/** What an agent connected over MCP may do: nothing, read, or also change data. */
export type AgentAccess = 'none' | 'read' | 'write'

/** What a connection's menu can create on its server. */
export type NewObjectKind = 'database' | 'role' | 'user'

/** A query language: an SQL dialect, mongosh for MongoDB, or Cosmos DB's SQL over JSON items. */
export type SqlDialect = 'postgres' | 'mysql' | 'sqlite' | 'sqlserver' | 'mongodb' | 'cosmosdb'

/** What the app knows of an engine. A new engine is one more entry in `ENGINES`. */
export interface EngineSpec {
  label: string
  /** Tables (in SQL), documents in collections or containers, or keys. */
  family: 'relational' | 'document' | 'keyValue'
  /** What a connection is: a server, a file (no host, port or user), or an account URI and key, as Azure gives them. */
  reach: 'server' | 'file' | 'account'
  defaultPort: number
  /** Connections can turn TLS on in the form. */
  tls: boolean
  /**
   * The schema to use when only a database is selected; `null` when databases
   * hold tables, collections or containers directly, with no schemas in between.
   */
  defaultSchema: string | null
  /** How queries are highlighted, completed and quoted. */
  dialect: SqlDialect
  /** Whole databases export to a .sql file and import back (pg_dump and psql). */
  dump: boolean
  /** What a connection's menu can create on its server, in menu order. */
  newObjects: NewObjectKind[]
  /** Its databases open in the key browser, which holds the CLI, rather than as tables and consoles. */
  keyBrowser: boolean
}

const SERVER_OBJECTS: NewObjectKind[] = ['database', 'role', 'user']

/** Every engine, in the order the connection form lists them. */
export const ENGINES: Record<Engine, EngineSpec> = {
  postgres: {
    label: 'PostgreSQL',
    family: 'relational',
    reach: 'server',
    defaultPort: 5432,
    tls: false,
    defaultSchema: 'public',
    dialect: 'postgres',
    dump: true,
    newObjects: SERVER_OBJECTS,
    keyBrowser: false,
  },
  mysql: {
    label: 'MySQL',
    family: 'relational',
    reach: 'server',
    defaultPort: 3306,
    tls: false,
    defaultSchema: null,
    dialect: 'mysql',
    dump: false,
    newObjects: SERVER_OBJECTS,
    keyBrowser: false,
  },
  mariadb: {
    label: 'MariaDB',
    family: 'relational',
    reach: 'server',
    defaultPort: 3306,
    tls: false,
    defaultSchema: null,
    dialect: 'mysql',
    dump: false,
    newObjects: SERVER_OBJECTS,
    keyBrowser: false,
  },
  sqlite: {
    label: 'SQLite',
    family: 'relational',
    reach: 'file',
    defaultPort: 0,
    tls: false,
    // The schema of the opened file.
    defaultSchema: 'main',
    dialect: 'sqlite',
    dump: false,
    newObjects: [],
    keyBrowser: false,
  },
  sqlserver: {
    label: 'SQL Server',
    family: 'relational',
    reach: 'server',
    defaultPort: 1433,
    tls: false,
    defaultSchema: 'dbo',
    dialect: 'sqlserver',
    dump: false,
    newObjects: SERVER_OBJECTS,
    keyBrowser: false,
  },
  mongodb: {
    label: 'MongoDB',
    family: 'document',
    reach: 'server',
    defaultPort: 27017,
    tls: true,
    defaultSchema: null,
    dialect: 'mongodb',
    dump: false,
    newObjects: SERVER_OBJECTS,
    keyBrowser: false,
  },
  cosmosdb: {
    label: 'Azure Cosmos DB',
    family: 'document',
    reach: 'account',
    defaultPort: 443,
    tls: true,
    defaultSchema: null,
    dialect: 'cosmosdb',
    dump: false,
    newObjects: ['database'],
    keyBrowser: false,
  },
  redis: {
    label: 'Redis',
    family: 'keyValue',
    reach: 'server',
    defaultPort: 6379,
    tls: true,
    defaultSchema: null,
    // Not queried from a console; the dialect only colours the CLI's commands.
    dialect: 'postgres',
    dump: false,
    newObjects: ['user'],
    keyBrowser: true,
  },
}

/** The engines, in the form's order. */
export const ENGINE_LIST = Object.keys(ENGINES) as Engine[]

/** A SQLite connection is a database file: no host, port or user. */
export function isFileEngine(engine: Engine): boolean {
  return ENGINES[engine].reach === 'file'
}

/** The database to use when none is selected; a SQLite file has only `main`. */
export function defaultDatabase(connection: Connection): string {
  return isFileEngine(connection.engine) ? ENGINES[connection.engine].defaultSchema! : connection.database
}

/** The schema to use when only a database is selected. */
export function defaultSchema(engine: Engine, database: string): string {
  return ENGINES[engine].defaultSchema ?? database
}

/** How queries are highlighted and completed for an engine. */
export function sqlDialectOf(engine: Engine | undefined): SqlDialect {
  return ENGINES[engine ?? 'postgres'].dialect
}

/** A dialect of SQL tables, as opposed to documents: the cell menu writes conditions and INSERTs in it. */
export function isRelationalDialect(dialect: SqlDialect): boolean {
  return ENGINE_LIST.some(
    (engine) => ENGINES[engine].dialect === dialect && ENGINES[engine].family === 'relational',
  )
}

/** Only SQL databases declare relationships between their tables, so only they have an ER diagram. */
export function hasDiagram(engine: Engine): boolean {
  return ENGINES[engine].family === 'relational'
}

/** Where a connection points: `user@host:port`, `host:port` for a key, or the file for SQLite. */
export function connectionAddress(connection: Connection, m: SharedMessages = messages.en): string {
  if (isFileEngine(connection.engine)) return connection.database
  if (ENGINES[connection.engine].reach === 'account') return `${connection.host}:${connection.port}`
  return `${connection.username || m.defaultUser}@${connection.host}:${connection.port}`
}
