export type Engine =
  'postgres' | 'mysql' | 'mariadb' | 'sqlite' | 'sqlserver' | 'mongodb' | 'redis' | 'cosmosdb'

export type Environment = 'production' | 'staging' | 'development'

/** A saved connection, as returned by the backend. The password is never included. */
export interface Connection {
  id: string
  name: string
  engine: Engine
  environment: Environment | null
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
}

/** Engines whose connections can use TLS from the form. */
export const TLS_ENGINES: Engine[] = ['redis', 'mongodb', 'cosmosdb']

/** Engines reached over TLS unless told otherwise: Azure serves Cosmos DB only over HTTPS. */
export const TLS_BY_DEFAULT: Engine[] = ['cosmosdb']

/** Engines that sign in with an account key instead of a user and password. */
export const KEY_ENGINES: Engine[] = ['cosmosdb']

export const ENGINE_LABELS: Record<Engine, string> = {
  postgres: 'PostgreSQL',
  mysql: 'MySQL',
  mariadb: 'MariaDB',
  sqlite: 'SQLite',
  sqlserver: 'SQL Server',
  mongodb: 'MongoDB',
  redis: 'Redis',
  cosmosdb: 'Azure Cosmos DB',
}

export const DEFAULT_PORTS: Record<Engine, number> = {
  postgres: 5432,
  mysql: 3306,
  mariadb: 3306,
  sqlite: 0,
  sqlserver: 1433,
  mongodb: 27017,
  redis: 6379,
  cosmosdb: 443,
}

export const RELATIONAL_ENGINES: Engine[] = ['postgres', 'mysql', 'mariadb', 'sqlite', 'sqlserver']
export const NOSQL_ENGINES: Engine[] = ['mongodb', 'cosmosdb', 'redis']

export const ENVIRONMENT_LABELS: Record<Environment, string> = {
  development: 'Dev',
  staging: 'Staging',
  production: 'Production',
}

/** Engines whose databases hold tables (or collections, or containers) directly, with no schemas in between. */
const SCHEMALESS: Engine[] = ['mysql', 'mariadb', 'mongodb', 'cosmosdb']

/** A SQLite connection is a database file: no host, port or user. */
export function isFileEngine(engine: Engine): boolean {
  return engine === 'sqlite'
}

/** SQLite's name for the schema of the opened file. */
const SQLITE_MAIN = 'main'

/** The database to use when none is selected; a SQLite file has only `main`. */
export function defaultDatabase(connection: Connection): string {
  return isFileEngine(connection.engine) ? SQLITE_MAIN : connection.database
}

/** The schema to use when only a database is selected. */
export function defaultSchema(engine: Engine, database: string): string {
  if (isFileEngine(engine)) return SQLITE_MAIN
  if (engine === 'sqlserver') return 'dbo'
  return SCHEMALESS.includes(engine) ? database : 'public'
}

/** A query language: an SQL dialect, mongosh for MongoDB, or Cosmos DB's SQL over JSON items. */
export type SqlDialect = 'postgres' | 'mysql' | 'sqlite' | 'sqlserver' | 'mongodb' | 'cosmosdb'

/** How queries are highlighted and completed for an engine. */
export function sqlDialectOf(engine: Engine | undefined): SqlDialect {
  if (engine === 'sqlite' || engine === 'sqlserver' || engine === 'mongodb' || engine === 'cosmosdb')
    return engine
  return engine && SCHEMALESS.includes(engine) ? 'mysql' : 'postgres'
}

/** MongoDB and Cosmos DB declare no relationships between collections, so they have no ER diagram. */
export function hasDiagram(engine: Engine): boolean {
  return RELATIONAL_ENGINES.includes(engine)
}

/** Where a connection points: `user@host:port`, `host:port` for a key, or the file for SQLite. */
export function connectionAddress(connection: Connection): string {
  if (isFileEngine(connection.engine)) return connection.database
  if (KEY_ENGINES.includes(connection.engine)) return `${connection.host}:${connection.port}`
  return `${connection.username || 'default user'}@${connection.host}:${connection.port}`
}
