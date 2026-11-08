// In-memory stand-in for the Rust backend, used only by `pnpm dev` in a plain
// browser so the UI can be developed without launching the desktop app.
import { mockIPC } from '@tauri-apps/api/mocks'
import type { Connection } from '../domain/connection'
import type { ConnectionInput } from '../features/connections/api'
import type { DbObject } from '../features/explorer/api'
import * as redis from './mockRedis'

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
    tls: false,
    allowSelfSigned: false,
    srv: false,
  },
  {
    id: 'demo-sqlite',
    name: 'local-sqlite',
    engine: 'sqlite',
    environment: 'development',
    host: '',
    port: 0,
    database: '/Users/demo/app.db',
    username: '',
    tls: false,
    allowSelfSigned: false,
    srv: false,
  },
  {
    id: 'demo-mysql',
    name: 'staging-mysql',
    engine: 'mysql',
    environment: 'staging',
    host: 'localhost',
    port: 3306,
    database: 'app',
    username: 'app',
    tls: false,
    allowSelfSigned: false,
    srv: false,
  },
  {
    id: 'demo-redis',
    name: 'cache-redis',
    engine: 'redis',
    environment: 'staging',
    host: 'cache.staging.internal',
    port: 6379,
    database: '0',
    username: '',
    tls: false,
    allowSelfSigned: false,
    srv: false,
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

/** A SQLite file lists its tables right under the connection. */
const sqliteObjects: Record<string, DbObject[]> = {
  '': [table('notes', 120), table('tags', 8), { name: 'recent_notes', kind: 'view', rowEstimate: null }],
}

/** MySQL has no schemas: a database lists its tables directly. */
const mysqlObjects: Record<string, DbObject[]> = {
  '': [{ name: 'app', kind: 'database', rowEstimate: null }],
  app: [
    table('customers', 48_210),
    table('orders', 612_400),
    { name: 'v_daily_sales', kind: 'view', rowEstimate: null },
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
        if (payload.connectionId === 'demo-redis') {
          const dbs = Array.from({ length: 16 }, (_, n): DbObject => ({
            name: `db${n}`,
            kind: 'database',
            rowEstimate: n === 0 ? redis.overview().keys : 0,
          }))
          return delay(path === '' ? dbs : [])
        }
        const source =
          payload.connectionId === 'demo-mysql'
            ? mysqlObjects
            : payload.connectionId === 'demo-sqlite'
              ? sqliteObjects
              : objects
        return delay(source[path] ?? [])
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
            id % 5 === 0 ? null : String(1000 + (id % 9)),
          ]
        })
        return delay({
          columns: [
            { name: 'id', dataType: 'integer', primaryKey: true },
            { name: 'name', dataType: 'character varying(80)', primaryKey: false },
            { name: 'revenue', dataType: 'numeric(10,2)', primaryKey: false },
            { name: 'created_at', dataType: 'timestamp with time zone', primaryKey: false, generated: true },
            { name: 'customer_id', dataType: 'bigint', primaryKey: false },
          ],
          rows,
          hasMore: offset + count < total,
        })
      }
      case 'redis_overview':
        return delay(redis.overview())
      case 'redis_scan':
        return delay(
          redis.scan(
            String(payload.pattern),
            payload.kind as string | null,
            String(payload.cursor),
            Number(payload.limit),
          ),
        )
      case 'redis_read_key':
        return delay(redis.readKey(String(payload.key), String(payload.filter)))
      case 'redis_edit_key':
        return delay(redis.editKey(String(payload.key), String(payload.kind), payload.edit as never))
      case 'redis_create_key':
        return delay(redis.createKey(payload.key as never))
      case 'run_query': {
        const sql = String(payload.sql)
        if (String(payload.sessionId).endsWith(':cli')) {
          const column = { name: 'reply', dataType: 'redis' }
          const rows = redis.cli(sql).map((line) => [line])
          return delay({
            elapsedMs: 1,
            results: [{ columns: [column], rows, truncated: false, rowsAffected: 1 }],
          })
        }
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
      case 'plugin:dialog|save':
        return delay(`/Users/demo/Downloads/${(payload.options as { defaultPath: string }).defaultPath}`)
      case 'export_table':
        return delay(48213)
      case 'export_rows':
        return delay((payload.rows as unknown[]).length)
      case 'format_rows':
        return delay('id,name\n1042,Northwind Traders\n')
      case 'list_history': {
        const search = String(payload.search).toLowerCase()
        return delay(
          mockHistory.filter(
            (entry) => entry.sql.toLowerCase().includes(search) && (!payload.errorsOnly || entry.error),
          ),
        )
      }
      case 'list_relations':
        return delay({
          database: payload.database ?? 'shop',
          relations: [
            { schema: 'analytics', name: 'v_orders_daily', kind: 'view' },
            { schema: 'public', name: 'customers', kind: 'table' },
            { schema: 'public', name: 'order_items', kind: 'table' },
            { schema: 'public', name: 'orders', kind: 'table' },
            { schema: 'public', name: 'payments', kind: 'table' },
            { schema: 'public', name: 'sales_order', kind: 'table' },
          ],
        })
      case 'apply_changes': {
        const changes = payload.changes as { kind: string; key?: Record<string, string> }[]
        if (changes.some((c) => c.key?.id === '13'))
          throw 'The row with id = 13 was changed or deleted since it was loaded. Nothing was applied; reload and try again.'
        const count = (kind: string) => changes.filter((c) => c.kind === kind).length
        return delay({ updated: count('update'), inserted: count('insert'), deleted: count('delete') })
      }
      case 'preview_changes':
        return delay(
          (payload.changes as { kind: string }[]).map((c) =>
            c.kind === 'delete'
              ? `DELETE FROM "public"."orders" WHERE "id" = CAST('3'::text AS integer)`
              : c.kind === 'insert'
                ? `INSERT INTO "public"."orders" ("name") VALUES (CAST('New'::text AS character varying(80)))`
                : `UPDATE "public"."orders" SET "name" = CAST('Renamed'::text AS character varying(80)) WHERE "id" = CAST('1'::text AS integer)`,
          ),
        )
      case 'schema_diagram':
        return delay(mockDiagram)
      case 'table_structure':
        return delay(mockStructure)
      case 'explain_query':
        return delay(payload.analyze ? mockPlan : mockPlanEstimate)
      case 'cancel_query':
      case 'close_session':
        sleeping.get(String(payload.sessionId))?.()
        sleeping.delete(String(payload.sessionId))
        return null
      case 'keep_cursor':
        return null
      default:
        throw new Error(`Mock backend: unknown command ${cmd}`)
    }
  })
}

const mockStructure = {
  kind: 'table',
  rowEstimate: 48213,
  dataBytes: 9_856_000,
  indexBytes: 3_250_000,
  comment: 'Paying customers',
  columns: [
    {
      name: 'id',
      dataType: 'bigint',
      nullable: false,
      default: 'GENERATED ALWAYS AS IDENTITY',
      comment: null,
    },
    {
      name: 'email',
      dataType: 'character varying(255)',
      nullable: false,
      default: null,
      comment: 'Login email',
    },
    { name: 'name', dataType: 'character varying(120)', nullable: false, default: null, comment: null },
    {
      name: 'country',
      dataType: 'character(2)',
      nullable: false,
      default: "'US'::bpchar",
      comment: 'ISO 3166-1',
    },
    { name: 'referrer_id', dataType: 'bigint', nullable: true, default: null, comment: null },
    {
      name: 'created_at',
      dataType: 'timestamp with time zone',
      nullable: false,
      default: 'now()',
      comment: null,
    },
  ],
  indexes: [
    {
      name: 'customers_pkey',
      method: 'btree',
      unique: true,
      primary: true,
      columns: ['id'],
      condition: null,
      bytes: 1_200_000,
    },
    {
      name: 'customers_email_key',
      method: 'btree',
      unique: true,
      primary: false,
      columns: ['email'],
      condition: null,
      bytes: 1_100_000,
    },
    {
      name: 'customers_referrer_idx',
      method: 'btree',
      unique: false,
      primary: false,
      columns: ['referrer_id'],
      condition: null,
      bytes: 200_000,
    },
  ],
  foreignKeys: [
    {
      name: 'orders_customer_id_fkey',
      columns: ['customer_id'],
      referencedTable: 'customers',
      referencedColumns: ['id'],
      onUpdate: 'NO ACTION',
      onDelete: 'RESTRICT',
    },
    {
      name: 'customers_referrer_id_fkey',
      columns: ['referrer_id'],
      referencedTable: 'customers',
      referencedColumns: ['id'],
      onUpdate: 'NO ACTION',
      onDelete: 'SET NULL',
    },
  ],
  triggers: [],
  ddl: `CREATE TABLE public.customers (
    id bigint GENERATED ALWAYS AS IDENTITY NOT NULL,
    email character varying(255) NOT NULL,
    name character varying(120) NOT NULL,
    country character(2) DEFAULT 'US'::bpchar NOT NULL,
    referrer_id bigint,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT customers_pkey PRIMARY KEY (id),
    CONSTRAINT customers_email_key UNIQUE (email),
    CONSTRAINT customers_referrer_id_fkey FOREIGN KEY (referrer_id) REFERENCES customers(id) ON DELETE SET NULL
);

CREATE INDEX customers_referrer_idx ON public.customers USING btree (referrer_id);

COMMENT ON TABLE public.customers IS 'Paying customers';`,
}

const hoursAgo = (hours: number) => Date.now() - hours * 60 * 60 * 1000
const mockHistory = [
  {
    sql: 'SELECT c.id, c.name, COUNT(o.id) AS orders\nFROM customers c JOIN orders o ON o.customer_id = c.id\nGROUP BY c.id',
    at: 0.1,
    ms: 38,
    rows: 14,
  },
  {
    sql: 'SELECT u.id, u.email, u.last_login FROM users u',
    at: 0.6,
    ms: 4,
    rows: 0,
    error: 'column u.last_login does not exist',
  },
  { sql: "UPDATE orders SET status = 'refunded' WHERE id = 98412", at: 1.2, ms: 6, rows: 1 },
  {
    sql: 'BEGIN; DELETE FROM sessions WHERE expires_at < now(); COMMIT',
    at: 26,
    ms: 420,
    rows: 0,
    statements: 3,
  },
  { sql: 'SELECT status, COUNT(*) FROM orders GROUP BY status', at: 27, ms: 210, rows: 5 },
  { sql: 'SELECT * FROM payments WHERE order_id = 98301', at: 80, ms: 2, rows: 1, connectionId: 'deleted' },
].map((entry, i) => ({
  id: String(i),
  executedAt: hoursAgo(entry.at),
  connectionId: entry.connectionId ?? connections[0].id,
  connectionName: entry.connectionId ? 'old-staging' : connections[0].name,
  database: 'shop',
  sql: entry.sql,
  elapsedMs: entry.ms,
  statements: entry.statements ?? 1,
  rows: entry.rows,
  error: entry.error ?? null,
}))

const mockPlan = [
  {
    Plan: {
      'Node Type': 'Sort',
      'Total Cost': 3120,
      'Plan Rows': 480,
      'Actual Total Time': 38.2,
      'Actual Rows': 14,
      'Actual Loops': 1,
      'Sort Key': ['(count(o.id)) DESC'],
      'Sort Method': 'quicksort',
      Plans: [
        {
          'Node Type': 'Aggregate',
          Strategy: 'Hashed',
          'Total Cost': 3080,
          'Plan Rows': 480,
          'Actual Total Time': 37.6,
          'Actual Rows': 14,
          'Actual Loops': 1,
          'Group Key': ['c.id'],
          Plans: [
            {
              'Node Type': 'Hash Join',
              'Join Type': 'Inner',
              'Total Cost': 2600,
              'Plan Rows': 51020,
              'Actual Total Time': 34.5,
              'Actual Rows': 48306,
              'Actual Loops': 1,
              'Hash Cond': '(o.customer_id = c.id)',
              Plans: [
                {
                  'Node Type': 'Seq Scan',
                  'Relation Name': 'orders',
                  Alias: 'o',
                  'Total Cost': 1800,
                  'Plan Rows': 1280,
                  'Actual Total Time': 24.1,
                  'Actual Rows': 48306,
                  'Actual Loops': 1,
                  Filter: "(created_at >= (now() - '30 days'::interval))",
                  'Rows Removed by Filter': 564102,
                },
                {
                  'Node Type': 'Hash',
                  'Total Cost': 700,
                  'Plan Rows': 48213,
                  'Actual Total Time': 4.4,
                  'Actual Rows': 48213,
                  'Actual Loops': 1,
                  Plans: [
                    {
                      'Node Type': 'Seq Scan',
                      'Relation Name': 'customers',
                      Alias: 'c',
                      'Total Cost': 650,
                      'Plan Rows': 48213,
                      'Actual Total Time': 3.2,
                      'Actual Rows': 48213,
                      'Actual Loops': 1,
                    },
                  ],
                },
              ],
            },
          ],
        },
      ],
    },
    'Planning Time': 0.9,
    'Execution Time': 38.4,
  },
]
const mockPlanEstimate = [
  {
    Plan: {
      'Node Type': 'Limit',
      'Total Cost': 120,
      'Plan Rows': 100,
      Plans: [
        {
          'Node Type': 'Seq Scan',
          'Relation Name': 'orders',
          Alias: 'orders',
          'Total Cost': 110,
          'Plan Rows': 100,
        },
      ],
    },
  },
]

const col = (name: string, dataType: string, primaryKey = false) => ({
  name,
  dataType,
  nullable: !primaryKey,
  primaryKey,
})
const fk = (table: string, column: string, referencedTable: string) => ({
  name: `${table}_${column}_fkey`,
  table,
  columns: [column],
  referencedSchema: 'public',
  referencedTable,
  referencedColumns: ['id'],
})
const mockDiagram = {
  tables: [
    {
      name: 'customers',
      rowEstimate: 48213,
      columns: [
        col('id', 'bigint', true),
        col('email', 'text'),
        col('name', 'text'),
        col('referrer_id', 'bigint'),
      ],
    },
    {
      name: 'order_items',
      rowEstimate: 1900000,
      columns: [
        col('id', 'bigint', true),
        col('order_id', 'bigint'),
        col('product_id', 'bigint'),
        col('qty', 'integer'),
      ],
    },
    {
      name: 'orders',
      rowEstimate: 612400,
      columns: [
        col('id', 'bigint', true),
        col('customer_id', 'bigint'),
        col('store_id', 'integer'),
        col('total', 'numeric(10,2)'),
        col('created_at', 'timestamp with time zone'),
      ],
    },
    {
      name: 'payments',
      rowEstimate: 588000,
      columns: [col('id', 'bigint', true), col('order_id', 'bigint'), col('amount', 'numeric(10,2)')],
    },
    {
      name: 'products',
      rowEstimate: 2400,
      columns: [col('id', 'bigint', true), col('sku', 'text'), col('price', 'numeric(10,2)')],
    },
    { name: 'stores', rowEstimate: 12, columns: [col('id', 'integer', true), col('name', 'text')] },
    { name: 'audit_log', rowEstimate: 90000, columns: [col('id', 'bigint', true), col('event', 'text')] },
  ],
  relationships: [
    fk('customers', 'referrer_id', 'customers'),
    fk('order_items', 'order_id', 'orders'),
    fk('order_items', 'product_id', 'products'),
    fk('orders', 'customer_id', 'customers'),
    fk('orders', 'store_id', 'stores'),
    fk('payments', 'order_id', 'orders'),
  ],
}
