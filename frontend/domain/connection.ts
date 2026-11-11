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
  /** How SSL/TLS is used and verified. */
  ssl: Ssl
  /** The SSH tunnel the server is reached through, when enabled. */
  ssh: SshTunnel
  /** MongoDB: the host names a DNS seed list (`mongodb+srv://`). */
  srv: boolean
  /** What AI agents connected over MCP may do with it. */
  agentAccess: AgentAccess
}

/**
 * How a connection uses SSL/TLS, from not at all to checking the server's
 * certificate and its name. The names follow PostgreSQL's `sslmode`.
 */
export type SslMode = 'disable' | 'prefer' | 'require' | 'verify-ca' | 'verify-full'

/** A connection's SSL/TLS settings. Files are PEM paths on this machine; empty when not used. */
export interface Ssl {
  mode: SslMode
  /** Authorities to trust besides the system's. */
  caFile: string
  /** The client's certificate. For MongoDB it holds the key too. */
  certFile: string
  /** The client certificate's private key; empty for MongoDB. */
  keyFile: string
}

/** How the SSH tunnel signs in: a password, a private key file, or the system's SSH agent. */
export type SshAuth = 'password' | 'key' | 'agent'

/** A tunnel through an SSH server that can reach the database. Its password or passphrase is never included. */
export interface SshTunnel {
  enabled: boolean
  host: string
  port: number
  username: string
  auth: SshAuth
  /** The private key, with `key` auth. */
  keyFile: string
}

/** What an engine's driver can do with SSL/TLS. */
export interface SslSupport {
  /** The modes it can use, from least to most checking. */
  modes: SslMode[]
  /** It can trust an authority from a file. */
  caFile: boolean
  /** It can present a client certificate. */
  clientCert: boolean
  /** The certificate and its key are separate files (MongoDB reads one file with both). */
  separateKey: boolean
  /** It can check the server's name through an SSH tunnel, whose local end is all the driver sees. */
  verifiesNameThroughSsh: boolean
}

/** An SSH tunnel that is off, as new connections and engines without a server have. */
export const NO_SSH_TUNNEL: SshTunnel = {
  enabled: false,
  host: '',
  port: 22,
  username: '',
  auth: 'password',
  keyFile: '',
}

const NO_SSL: SslSupport = {
  modes: ['disable'],
  caFile: false,
  clientCert: false,
  separateKey: false,
  verifiesNameThroughSsh: false,
}

const SQL_SSL: SslSupport = {
  modes: ['disable', 'prefer', 'require', 'verify-ca', 'verify-full'],
  caFile: true,
  clientCert: true,
  separateKey: true,
  verifiesNameThroughSsh: false,
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
  /** The SSL/TLS settings its driver can apply. */
  ssl: SslSupport
  /** Its server can be reached through an SSH tunnel: not a file, nor an account URL. */
  tunnels: boolean
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
  /** A database's menu can drop it from the server. */
  dropsDatabases: boolean
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
    ssl: SQL_SSL,
    tunnels: true,
    defaultSchema: 'public',
    dialect: 'postgres',
    dump: true,
    newObjects: SERVER_OBJECTS,
    dropsDatabases: true,
    keyBrowser: false,
  },
  mysql: {
    label: 'MySQL',
    family: 'relational',
    reach: 'server',
    defaultPort: 3306,
    ssl: SQL_SSL,
    tunnels: true,
    defaultSchema: null,
    dialect: 'mysql',
    dump: false,
    newObjects: SERVER_OBJECTS,
    dropsDatabases: true,
    keyBrowser: false,
  },
  mariadb: {
    label: 'MariaDB',
    family: 'relational',
    reach: 'server',
    defaultPort: 3306,
    ssl: SQL_SSL,
    tunnels: true,
    defaultSchema: null,
    dialect: 'mysql',
    dump: false,
    newObjects: SERVER_OBJECTS,
    dropsDatabases: true,
    keyBrowser: false,
  },
  sqlite: {
    label: 'SQLite',
    family: 'relational',
    reach: 'file',
    defaultPort: 0,
    ssl: NO_SSL,
    tunnels: false,
    // The schema of the opened file.
    defaultSchema: 'main',
    dialect: 'sqlite',
    dump: false,
    newObjects: [],
    dropsDatabases: false,
    keyBrowser: false,
  },
  sqlserver: {
    label: 'SQL Server',
    family: 'relational',
    reach: 'server',
    defaultPort: 1433,
    ssl: {
      modes: ['disable', 'prefer', 'require', 'verify-full'],
      caFile: true,
      clientCert: true,
      separateKey: true,
      // Its driver checks the certificate's name apart from the address it connects to.
      verifiesNameThroughSsh: true,
    },
    tunnels: true,
    defaultSchema: 'dbo',
    dialect: 'sqlserver',
    dump: false,
    newObjects: SERVER_OBJECTS,
    dropsDatabases: true,
    keyBrowser: false,
  },
  mongodb: {
    label: 'MongoDB',
    family: 'document',
    reach: 'server',
    defaultPort: 27017,
    ssl: {
      modes: ['disable', 'require', 'verify-full'],
      caFile: true,
      clientCert: true,
      separateKey: false,
      verifiesNameThroughSsh: false,
    },
    tunnels: true,
    defaultSchema: null,
    dialect: 'mongodb',
    dump: false,
    newObjects: SERVER_OBJECTS,
    dropsDatabases: true,
    keyBrowser: false,
  },
  cosmosdb: {
    label: 'Azure Cosmos DB',
    family: 'document',
    reach: 'account',
    defaultPort: 443,
    ssl: { ...NO_SSL, modes: ['disable', 'require', 'verify-full'] },
    tunnels: false,
    defaultSchema: null,
    dialect: 'cosmosdb',
    dump: false,
    newObjects: ['database'],
    dropsDatabases: true,
    keyBrowser: false,
  },
  redis: {
    label: 'Redis',
    family: 'keyValue',
    reach: 'server',
    defaultPort: 6379,
    ssl: {
      modes: ['disable', 'require', 'verify-full'],
      caFile: true,
      clientCert: true,
      separateKey: true,
      verifiesNameThroughSsh: false,
    },
    tunnels: true,
    defaultSchema: null,
    // Not queried from a console; the dialect only colours the CLI's commands.
    dialect: 'postgres',
    dump: false,
    newObjects: ['user'],
    dropsDatabases: false,
    keyBrowser: true,
  },
}

/** The engines, in the form's order. */
export const ENGINE_LIST = Object.keys(ENGINES) as Engine[]

/** A SQLite connection is a database file: no host, port or user. */
export function isFileEngine(engine: Engine): boolean {
  return ENGINES[engine].reach === 'file'
}

/** The SSL mode a new connection starts with: `prefer` where the engine has it, else no SSL. */
export function defaultSslMode(engine: Engine): SslMode {
  return ENGINES[engine].ssl.modes.includes('prefer') ? 'prefer' : 'disable'
}

/** The mode encrypts the connection, at least when the server allows it. */
export function encrypts(mode: SslMode): boolean {
  return mode !== 'disable'
}

/**
 * Through an SSH tunnel the driver only sees the tunnel's local address, so
 * only some can check the certificate names the server.
 */
export function sslModeWorksThroughSsh(engine: Engine, mode: SslMode): boolean {
  return mode !== 'verify-full' || ENGINES[engine].ssl.verifiesNameThroughSsh
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
