// In-memory stand-in for the Rust backend, used only by `pnpm dev` in a plain
// browser so the UI can be developed without launching the desktop app.
import type { Text } from '../features/keyspace/api'
import { emit } from '@tauri-apps/api/event'
import { clearRecordedStatements, recordMock, recordedStatements } from './mockStatements'
import { mockBackup } from './mockBackup'
import { exposeMockApproval, mockMcp } from './mockMcp'
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
    agentAccess: 'write',
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
    agentAccess: 'read',
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
    agentAccess: 'none',
  },
  {
    id: 'demo-cosmos',
    name: 'orders-cosmos',
    engine: 'cosmosdb',
    environment: 'development',
    host: 'demo.documents.azure.com',
    port: 443,
    database: '',
    username: '',
    tls: true,
    allowSelfSigned: false,
    srv: false,
    agentAccess: 'none',
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
    agentAccess: 'none',
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
  exposeMockApproval()
  mockIPC(
    async (cmd, args) => {
      const payload = args as Record<string, unknown>
      try {
        const result = await answer(cmd, payload)
        void recordMock(cmd, payload)
        return result
      } catch (error) {
        void recordMock(cmd, payload, String(error))
        throw error
      }
    },
    { shouldMockEvents: true },
  )
}

/** The mock's answer to each command, as the real backend would give it. */
async function answer(cmd: string, payload: Record<string, unknown>): Promise<unknown> {
  const mcp = mockMcp(cmd, payload)
  if (mcp.handled) return delay(mcp.value)
  const backup = mockBackup(cmd)
  if (backup.handled) return delay(backup.value)
  switch (cmd) {
    case 'list_statements':
      return delay(recordedStatements())
    case 'clear_statements':
      clearRecordedStatements()
      return null
    case 'list_connections':
      // A copy, as the real backend's JSON is: React must never share the mock's array.
      return delay([...connections])
    case 'save_connection': {
      const { password: _password, ...input } = payload.input as ConnectionInput
      const saved = { ...input, id: input.id ?? crypto.randomUUID() }
      const index = connections.findIndex((c) => c.id === saved.id)
      if (index >= 0) connections[index] = saved
      else connections.push(saved)
      return delay(saved)
    }
    case 'duplicate_connection': {
      const original = connections.find((c) => c.id === payload.id)!
      const copy = { ...original, id: crypto.randomUUID(), name: String(payload.name) }
      connections.push(copy)
      return delay(copy)
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
      if (payload.connectionId === 'demo-cosmos') {
        const tree: Record<string, DbObject[]> = {
          '': [{ name: 'shop', kind: 'database', rowEstimate: null }],
          shop: [table('order-items', 120)],
        }
        return delay(tree[path] ?? [])
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
      if (payload.connectionId === 'demo-cosmos') return delay(cosmosPage(payload.offset as number))
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
          `00000000-0000-4000-8000-${String(id).padStart(12, '0')}`,
          id % 6 === 0
            ? null
            : `2026-${String((id % 12) + 1).padStart(2, '0')}-${String((id % 28) + 1).padStart(2, '0')}`,
          `2026-09-16 ${String(id % 24).padStart(2, '0')}:00:27`,
          id % 4 === 0 ? 'false' : 'true',
          id % 9 === 0 ? null : ['lead', 'trial', 'paying', 'churned'][id % 4],
        ]
      })
      return delay({
        columns: [
          { name: 'id', dataType: 'integer', primaryKey: true },
          { name: 'name', dataType: 'character varying(80)', primaryKey: false },
          { name: 'revenue', dataType: 'numeric(10,2)', primaryKey: false },
          {
            name: 'created_at',
            dataType: 'timestamp with time zone',
            primaryKey: false,
            generated: true,
          },
          { name: 'customer_id', dataType: 'bigint', primaryKey: false },
          { name: 'external_id', dataType: 'uuid', primaryKey: false },
          { name: 'birthday', dataType: 'date', primaryKey: false },
          { name: 'last_login', dataType: 'timestamp without time zone', primaryKey: false },
          { name: 'active', dataType: 'boolean', primaryKey: false },
          { name: 'status', dataType: 'customer_status', primaryKey: false },
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
      return delay(redis.readKey((payload.key as Text).text, String(payload.filter)))
    case 'redis_edit_key':
      return delay(redis.editKey((payload.key as Text).text, String(payload.kind), payload.edit as never))
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
          error: null,
        })
      }
      if (/^\s*error/i.test(sql)) throw 'syntax error at or near "error"'
      // A later statement failing: what ran before it is shown with the error.
      if (/error/i.test(sql)) {
        return delay({
          elapsedMs: 12,
          results: [{ columns: [], rows: [], truncated: false, rowsAffected: 3 }],
          error: 'syntax error at or near "error"',
        })
      }
      if (/pg_sleep/i.test(sql)) {
        await new Promise<void>((resolve) => sleeping.set(String(payload.sessionId), resolve))
        throw 'canceling statement due to user request'
      }
      return delay({
        elapsedMs: 38,
        error: null,
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
    case 'export_table': {
      // 48,213 rows in ten steps, a cancel ending it between two.
      const jobId = String(payload.jobId)
      const total = 48213
      for (let step = 1; step <= 10; step++) {
        await new Promise((resolve) => setTimeout(resolve, 300))
        if (cancelled.delete(jobId)) throw 'Cancelled.'
        await emit('export-progress', { jobId, rows: Math.round((total * step) / 10) })
      }
      return total
    }
    case 'reveal_file':
      return null
    case 'export_rows':
      return delay((payload.rows as unknown[]).length)
    case 'format_rows':
      return delay('id,name\n1042,Northwind Traders\n')
    case 'plugin:updater|check':
      // A newer version, so the update dialog can be seen.
      return delay({
        rid: 1,
        currentVersion: '0.1.0',
        version: '0.2.0',
        date: '2026-11-12 10:00:00.0 +00:00:00',
        body: '## Features\n\n- **import:** import rows from CSV, TSV and JSON files\n- **export:** format options, column choice, progress and stop\n\n## Fixes\n\n- **grid:** the row number header is centered like the column headers\n',
        rawJson: {},
      })
    case 'plugin:updater|download_and_install':
      return delay(null)
    case 'plugin:process|restart':
      return null
    case 'set_aside_files':
      return []
    case 'list_history': {
      const search = String(payload.search).toLowerCase()
      return delay(
        mockHistory.filter(
          (entry) =>
            entry.sql.toLowerCase().includes(search) &&
            (!payload.connectionId || entry.connectionId === payload.connectionId) &&
            (!payload.errorsOnly || entry.error),
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
      const count = (...kinds: string[]) => changes.filter((c) => kinds.includes(c.kind)).length
      return delay({
        updated: count('update', 'replace'),
        inserted: count('insert', 'insertDocument'),
        deleted: count('delete'),
      })
    }
    case 'preview_changes':
      return delay(
        (payload.changes as { kind: string; document?: string }[]).map((c) =>
          c.document !== undefined
            ? cosmosWrite(c.kind, c.document)
            : c.kind === 'delete'
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
    case 'plugin:dialog|open': {
      // A data file when the dialog asks for one, else a dump.
      const filters = (payload.options as { filters?: { extensions: string[] }[] }).filters ?? []
      const data = filters.some((filter) => filter.extensions.includes('csv'))
      return delay(data ? '/Users/demo/Downloads/customers.csv' : '/Users/demo/Downloads/shop.sql')
    }
    case 'preview_import': {
      if (payload.format === 'json') {
        return delay({
          fields: ['id', 'name', 'email', 'tags'],
          rows: [
            ['9001', 'Ada', 'ada@example.com', '["vip"]'],
            ['9002', 'Bo', null, null],
          ],
        })
      }
      const header = (payload.options as { header: boolean }).header
      const rows = [
        ['9001', 'Ada Lovelace', 'ada@example.com', '120.50'],
        ['9002', 'Bo Diddley', null, '7.00'],
        ['9003', 'Cy Twombly', 'cy@example.com', '0'],
      ]
      return delay({
        fields: header
          ? ['id', 'Name', 'email', 'lifetime_value']
          : ['Column 1', 'Column 2', 'Column 3', 'Column 4'],
        rows: header ? rows : [['id', 'Name', 'email', 'lifetime_value'], ...rows],
      })
    }
    case 'import_table': {
      // 12,480 rows in eight steps, a cancel ending it between two.
      const { jobId } = payload.request as { jobId: string }
      const total = 12480
      for (let step = 1; step <= 8; step++) {
        await new Promise((resolve) => setTimeout(resolve, 300))
        if (cancelled.delete(jobId)) throw 'Cancelled.'
        await emit('import-progress', { jobId, rows: Math.round((total * step) / 8) })
      }
      return total
    }
    case 'export_database': {
      const { jobId, database } = payload.request as { jobId: string; database: string }
      await toolRun(jobId, [
        'pg_dump: last built-in OID is 16383',
        'pg_dump: reading user-defined tables',
        `pg_dump: dumping contents of table "public.orders"`,
        `pg_dump: dumping contents of table "public.customers"`,
      ])
      if (database === 'broken')
        throw 'pg_dump: error: connection to server failed: password authentication failed'
      return { bytes: 9_812_345, elapsedMs: 2400 }
    }
    case 'import_database': {
      const { jobId, database } = payload.request as { jobId: string; database: string }
      await toolRun(jobId, ['SET', 'CREATE TABLE', 'COPY 450', 'ALTER TABLE', 'CREATE INDEX'])
      return { created: !objects[''].some((o) => o.name === database), elapsedMs: 1830 }
    }
    case 'create_object': {
      const { object } = payload.request as { object: { kind: string; name: string } }
      const taken = object.kind === 'database' ? objects[''].map((o) => o.name) : [...accounts]
      if (taken.includes(object.name)) throw `${object.kind} "${object.name}" already exists`
      if (object.kind === 'database')
        objects[''].push({ name: object.name, kind: 'database', rowEstimate: null })
      else accounts.add(object.name)
      return delay(null)
    }
    case 'list_tags':
      return delay(tags)
    case 'save_tags': {
      const list = payload.list as typeof tags
      const names = list.map((tag) => tag.name.trim().toLowerCase())
      if (names.some((name) => !name)) throw 'Every tag needs a name.'
      if (new Set(names).size < names.length) throw 'There is already a tag with that name.'
      const kept = new Set(list.map((tag) => tag.id))
      for (const connection of connections)
        if (connection.environment && !kept.has(connection.environment)) connection.environment = null
      tags = list.map((tag) => ({ ...tag, name: tag.name.trim() }))
      return delay(tags)
    }
    case 'credential_storage':
      return delay(credentials)
    case 'set_credential_storage': {
      for (let current = 1; current <= connections.length; current++) {
        await emit('credential-progress', { phase: 'moving', current, total: connections.length })
        await new Promise((resolve) => setTimeout(resolve, 400))
      }
      credentials = { ...credentials, storage: payload.storage as 'keychain' | 'file' }
      return delay(credentials)
    }
    case 'plugin:app|version':
      return '0.1.0'
    case 'dump_tool':
      return delay(mockTools[String(payload.name)])
    case 'set_dump_tool': {
      const name = String(payload.name)
      const path = payload.path as string | null
      if (path && !/pg_dump|psql/.test(path))
        throw `The ${name} chosen in DBForge can't be run: ${path}. Choose another one, or use the one DBForge finds.`
      mockTools[name] = path ? { path, version: '16.4', custom: true } : { ...foundTools[name] }
      return delay(mockTools[name])
    }
    case 'export_folder_usage':
      return delay(exportFiles)
    case 'clear_export_folder': {
      const removed = exportFiles
      exportFiles = { count: 0, bytes: 0 }
      return delay(removed)
    }
    case 'dump_file_exists':
      return delay(String(payload.path).includes('exists'))
    case 'plugin:path|resolve_directory':
      return '/Users/demo/Documents'
    case 'cancel_job':
      cancelled.add(String(payload.jobId))
      return null
    default:
      throw new Error(`Mock backend: unknown command ${cmd}`)
  }
}

/** Jobs the dialogs asked to stop. */
const cancelled = new Set<string>()
// The PostgreSQL client tools found, and the ones chosen in their place.
const foundTools: Record<string, { path: string; version: string; custom: boolean }> = {
  pg_dump: { path: '/opt/homebrew/opt/postgresql@18/bin/pg_dump', version: '18.1', custom: false },
  psql: { path: '/opt/homebrew/opt/postgresql@18/bin/psql', version: '18.1', custom: false },
}
const mockTools = { ...foundTools }
// The connection tags, as the settings edit them.
let tags = [
  { id: 'production', name: 'PROD', color: 'red', confirm: true },
  { id: 'staging', name: 'STAGE', color: 'amber', confirm: false },
  { id: 'test', name: 'TEST', color: 'violet', confirm: false },
  { id: 'development', name: 'DEV', color: 'blue', confirm: false },
]
// Where passwords are kept, as the settings choose.
let credentials = {
  storage: 'keychain' as 'keychain' | 'file',
  filePath: '/Users/demo/Library/Application Support/dev.dbforge.app/credentials.bin',
}
// The exports in the export folder.
let exportFiles = { count: 7, bytes: 48_213_504 }
// Roles and users created from a connection's menu.
const accounts = new Set(['postgres'])

/** A pg_dump or psql run: its lines arrive one by one, and a cancel stops it. */
async function toolRun(jobId: string, lines: string[]) {
  for (const line of lines) {
    await new Promise((resolve) => setTimeout(resolve, 500))
    if (cancelled.delete(jobId)) throw 'Cancelled.'
    await emit('dump-progress', { jobId, line })
  }
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
    { name: 'revenue', dataType: 'numeric(10,2)', nullable: true, default: '0', comment: null },
    { name: 'customer_id', dataType: 'bigint', nullable: true, default: null, comment: null },
    {
      name: 'external_id',
      dataType: 'uuid',
      nullable: false,
      default: 'gen_random_uuid()',
      comment: null,
    },
    {
      name: 'created_at',
      dataType: 'timestamp with time zone',
      nullable: false,
      default: 'now()',
      comment: null,
    },
    { name: 'active', dataType: 'boolean', nullable: false, default: 'true', comment: null },
    {
      name: 'status',
      dataType: 'customer_status',
      nullable: true,
      default: null,
      comment: null,
      options: ['lead', 'trial', 'paying', 'churned'],
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

/** Items of the demo Cosmos DB container, as the driver sends them: rows and their documents. */
function cosmosPage(offset: number) {
  const total = 120
  const items = Array.from({ length: Math.max(0, Math.min(50, total - offset)) }, (_, i) => {
    const n = offset + i + 1
    return {
      id: `order-${n}`,
      customer: `c${n % 4}`,
      total: n * 10,
      paid: n % 3 !== 0,
      items: [{ sku: `sku-${n}`, qty: (n % 3) + 1 }],
    }
  })
  return {
    columns: [
      { name: 'id', dataType: 'string', primaryKey: true },
      { name: 'customer', dataType: 'string', primaryKey: true },
      { name: 'total', dataType: 'number', primaryKey: false },
      { name: 'paid', dataType: 'boolean', primaryKey: false },
      { name: 'items', dataType: 'array', primaryKey: false },
    ],
    rows: items.map((item) => [
      item.id,
      item.customer,
      String(item.total),
      String(item.paid),
      JSON.stringify(item.items),
    ]),
    documents: items.map((item) => JSON.stringify(item, null, 2)),
    hasMore: offset + items.length < total,
  }
}

/** A whole-document write as the Cosmos DB driver previews it, refusing what isn't JSON. */
function cosmosWrite(kind: string, document: string) {
  let item: unknown
  try {
    item = JSON.parse(document)
  } catch (error) {
    throw `The document isn't valid JSON: ${error instanceof Error ? error.message : error}.`
  }
  if (typeof item !== 'object' || item === null || Array.isArray(item))
    throw 'A document is a JSON object, in braces.'
  const text = JSON.stringify(item)
  return kind === 'replace'
    ? `REPLACE order-items PARTITION ["c1"] ${text}`
    : `CREATE order-items PARTITION ["c1"] ${text}`
}
