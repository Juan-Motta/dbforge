export type Engine = 'postgres' | 'mysql' | 'mariadb' | 'sqlite' | 'sqlserver' | 'mongodb' | 'redis'

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
}

export const ENGINE_LABELS: Record<Engine, string> = {
  postgres: 'PostgreSQL',
  mysql: 'MySQL',
  mariadb: 'MariaDB',
  sqlite: 'SQLite',
  sqlserver: 'SQL Server',
  mongodb: 'MongoDB',
  redis: 'Redis',
}

export const DEFAULT_PORTS: Record<Engine, number> = {
  postgres: 5432,
  mysql: 3306,
  mariadb: 3306,
  sqlite: 0,
  sqlserver: 1433,
  mongodb: 27017,
  redis: 6379,
}

export const RELATIONAL_ENGINES: Engine[] = ['postgres', 'mysql', 'mariadb', 'sqlite', 'sqlserver']
export const NOSQL_ENGINES: Engine[] = ['mongodb', 'redis']

export const ENVIRONMENT_LABELS: Record<Environment, string> = {
  development: 'Dev',
  staging: 'Staging',
  production: 'Production',
}

/** Engines whose databases hold tables directly, with no schemas in between. */
const SCHEMALESS: Engine[] = ['mysql', 'mariadb']

/** The schema to use when only a database is selected. */
export function defaultSchema(engine: Engine, database: string): string {
  return SCHEMALESS.includes(engine) ? database : 'public'
}

export type SqlDialect = 'postgres' | 'mysql'

/** How SQL is highlighted and completed for an engine. */
export function sqlDialectOf(engine: Engine | undefined): SqlDialect {
  return engine && SCHEMALESS.includes(engine) ? 'mysql' : 'postgres'
}
