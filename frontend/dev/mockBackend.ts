// In-memory stand-in for the Rust backend, used only by `pnpm dev` in a plain
// browser so the UI can be developed without launching the desktop app.
import { mockIPC } from '@tauri-apps/api/mocks'
import type { Connection } from '../domain/connection'
import type { ConnectionInput } from '../features/connections/api'
import type { DbObject } from '../features/explorer/api'

const connections: Connection[] = [
  {
    id: 'demo',
    name: 'demo-postgres',
    engine: 'postgres',
    environment: 'production',
    host: 'localhost',
    port: 5432,
    database: 'shop',
    username: 'app',
  },
]

const table = (name: string, rowEstimate: number): DbObject => ({ name, kind: 'table', rowEstimate })

const objects: Record<string, DbObject[]> = {
  '': [
    { name: 'postgres', kind: 'database', rowEstimate: null },
    { name: 'shop', kind: 'database', rowEstimate: null },
  ],
  shop: [
    { name: 'analytics', kind: 'schema', rowEstimate: null },
    { name: 'public', kind: 'schema', rowEstimate: null },
  ],
  'shop/public': [
    table('customers', 48_210),
    table('orders', 612_400),
    table('payments', 588_000),
    { name: 'active_customers', kind: 'view', rowEstimate: null },
  ],
}

const delay = <T>(value: T) => new Promise<T>((resolve) => setTimeout(() => resolve(value), 300))

/** Queries containing pg_sleep wait here until cancelled, by session id. */
const sleeping = new Map<string, () => void>()

export function installMockBackend() {
  mockIPC(async (cmd, args) => {
    const payload = args as Record<string, unknown>
    switch (cmd) {
      case 'list_connections':
        return delay(connections)
      case 'save_connection': {
        const { password: _password, ...input } = payload.input as ConnectionInput
        const saved = { ...input, id: input.id ?? crypto.randomUUID() }
        const index = connections.findIndex((c) => c.id === saved.id)
        if (index >= 0) connections[index] = saved
        else connections.push(saved)
        return delay(saved)
      }
      case 'delete_connection':
        connections.splice(
          connections.findIndex((c) => c.id === payload.id),
          1,
        )
        return delay(null)
      case 'test_connection':
        return delay('PostgreSQL 17.0 (mock)')
      case 'list_objects': {
        const path = (payload.path as string[]).join('/')
        return delay(objects[path] ?? [])
      }
      case 'fetch_rows': {
        const offset = payload.offset as number
        const limit = payload.limit as number
        const total = 450
        const count = Math.max(0, Math.min(limit, total - offset))
        const rows = Array.from({ length: count }, (_, i) => {
          const id = offset + i + 1
          return [
            String(id),
            `Customer ${id}`,
            id % 7 === 0 ? null : (id * 13.37).toFixed(2),
            '2026-11-05 10:00:00+00',
          ]
        })
        return delay({
          columns: [
            { name: 'id', dataType: 'integer', primaryKey: true },
            { name: 'name', dataType: 'character varying(80)', primaryKey: false },
            { name: 'revenue', dataType: 'numeric(10,2)', primaryKey: false },
            { name: 'created_at', dataType: 'timestamp with time zone', primaryKey: false },
          ],
          rows,
          hasMore: offset + count < total,
        })
      }
      case 'run_query': {
        const sql = String(payload.sql)
        if (/error/i.test(sql)) throw 'syntax error at or near "error"'
        if (/pg_sleep/i.test(sql)) {
          await new Promise<void>((resolve) => sleeping.set(String(payload.sessionId), resolve))
          throw 'canceling statement due to user request'
        }
        return delay({
          elapsedMs: 38,
          results: [
            { columns: [], rows: [], truncated: false, rowsAffected: 0 },
            {
              columns: [
                { name: 'id', dataType: 'int4' },
                { name: 'name', dataType: 'text' },
                { name: 'revenue', dataType: 'numeric' },
              ],
              rows: [
                ['1042', 'Northwind Traders', '12480.00'],
                ['2210', 'Blue Harbor Coffee', null],
              ],
              truncated: false,
              rowsAffected: 2,
            },
          ],
        })
      }
      case 'cancel_query':
      case 'close_session':
        sleeping.get(String(payload.sessionId))?.()
        sleeping.delete(String(payload.sessionId))
        return null
      default:
        throw new Error(`Mock backend: unknown command ${cmd}`)
    }
  })
}
