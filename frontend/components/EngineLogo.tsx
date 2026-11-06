import type { Engine } from '../domain/connection'
import { ENGINE_LABELS } from '../domain/connection'
import postgres from '../assets/engines/postgresql.svg'
import mysql from '../assets/engines/mysql.svg'
import mariadb from '../assets/engines/mariadb.svg'
import sqlite from '../assets/engines/sqlite.svg'
import sqlserver from '../assets/engines/sql-server.svg'
import mongodb from '../assets/engines/mongodb.svg'
import redis from '../assets/engines/redis.svg'

const LOGOS: Record<Engine, string> = {
  postgres,
  mysql,
  mariadb,
  sqlite,
  sqlserver,
  mongodb,
  redis,
}

export function EngineLogo({ engine, size = 16 }: { engine: Engine; size?: number }) {
  return (
    <img
      src={LOGOS[engine]}
      alt={ENGINE_LABELS[engine]}
      width={size}
      height={size}
      draggable={false}
      style={{ flex: 'none' }}
    />
  )
}
