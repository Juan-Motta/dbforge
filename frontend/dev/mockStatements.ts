import { emit } from '@tauri-apps/api/event'
import type { Origin, StatementEntry } from '../features/statement-log/api'

/** What the mock backend "ran", as the real one records it: kept and sent as events. */
let recorded: StatementEntry[] = []
let nextId = 1

type Payload = Record<string, unknown>
type Table = { database: string; schema: string; name: string }

/** The statements a command would send, by command, in the engine's language. */
function statementsOf(cmd: string, payload: Payload): { origin: Origin; database?: string; text: string }[] {
  const table = payload.table as Table | undefined
  const name = table && `"${table.schema}"."${table.name}"`
  switch (cmd) {
    case 'list_objects':
      return [
        {
          origin: 'metadata',
          text: 'SELECT datname FROM pg_database WHERE NOT datistemplate AND datallowconn ORDER BY datname',
        },
      ]
    case 'fetch_rows':
      return [
        {
          origin: 'browse',
          database: table?.database,
          text: `SELECT * FROM ${name} ORDER BY "id" LIMIT ${Number(payload.limit) + 1} OFFSET ${payload.offset}`,
        },
      ]
    case 'table_structure':
      return [
        {
          origin: 'metadata',
          database: table?.database,
          text: `SELECT a.attname, format_type(a.atttypid, a.atttypmod) FROM pg_attribute a WHERE a.attrelid = '${table?.schema}.${table?.name}'::regclass`,
        },
      ]
    case 'run_query':
      return [{ origin: 'console', text: String(payload.sql) }]
    case 'apply_changes':
      return [
        { origin: 'edit', database: table?.database, text: 'BEGIN' },
        {
          origin: 'edit',
          database: table?.database,
          text: `UPDATE ${name} SET "status" = CAST('paying'::text AS "public"."status") WHERE "id" = CAST('2'::text AS pg_catalog.int4)`,
        },
        { origin: 'edit', database: table?.database, text: 'COMMIT' },
      ]
    default:
      return []
  }
}

/** Records what `cmd` would run; `error` marks the last statement as failed. */
export async function recordMock(cmd: string, payload: Payload, error?: string) {
  const connectionId = String(payload.connectionId ?? '')
  const statements = statementsOf(cmd, payload)
  for (const [index, statement] of statements.entries()) {
    const failed = error !== undefined && index === statements.length - 1
    const entry: StatementEntry = {
      id: nextId++,
      at: Date.now(),
      connectionId,
      database: statement.database ?? null,
      origin: statement.origin,
      text: statement.text,
      elapsedMs: 2 + Math.round(Math.random() * 30),
      rows: failed ? null : statement.origin === 'edit' && statement.text.startsWith('UPDATE') ? 1 : 50,
      error: failed ? error : null,
    }
    recorded = [...recorded, entry].slice(-2000)
    await emit('statement-logged', entry)
  }
}

export function recordedStatements() {
  return recorded
}

export function clearRecordedStatements() {
  recorded = []
}
