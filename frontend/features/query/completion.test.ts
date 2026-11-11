import { describe, expect, it } from 'vitest'
import { changesStructure, completionNames, type CompletionSchema } from './completion'

const shop: CompletionSchema = {
  database: 'shop',
  tables: [
    { schema: 'public', name: 'orders', kind: 'table', columns: [{ name: 'id', dataType: 'integer' }] },
    { schema: 'public', name: 'open_orders', kind: 'view', columns: [{ name: 'id', dataType: 'integer' }] },
    {
      schema: 'billing',
      name: 'invoices',
      kind: 'table',
      columns: [{ name: 'total', dataType: 'numeric(10,2)' }],
    },
  ],
}

describe('completionNames', () => {
  it('nests columns in tables in schemas, saying what each one is', () => {
    const { namespace, defaultSchema } = completionNames(shop, 'postgres')
    expect(defaultSchema).toBe('public')
    expect(namespace).toEqual({
      public: {
        self: { label: 'public', type: 'namespace', detail: 'schema' },
        children: {
          orders: {
            self: { label: 'orders', type: 'type', detail: 'table' },
            children: [{ label: 'id', type: 'property', detail: 'integer' }],
          },
          open_orders: {
            self: { label: 'open_orders', type: 'type', detail: 'view' },
            children: [{ label: 'id', type: 'property', detail: 'integer' }],
          },
        },
      },
      billing: {
        self: { label: 'billing', type: 'namespace', detail: 'schema' },
        children: {
          invoices: {
            self: { label: 'invoices', type: 'type', detail: 'table' },
            children: [{ label: 'total', type: 'property', detail: 'numeric(10,2)' }],
          },
        },
      },
    })
  })

  it("takes MySQL's database as the schema typed without its name", () => {
    const mysql: CompletionSchema = { database: 'app', tables: [{ ...shop.tables[0], schema: 'app' }] }
    expect(completionNames(mysql, 'mysql').defaultSchema).toBe('app')
  })

  it('names no default schema the database lacks', () => {
    expect(
      completionNames({ database: 'shop', tables: [shop.tables[2]] }, 'postgres').defaultSchema,
    ).toBeUndefined()
  })
})

describe('changesStructure', () => {
  it('spots statements that create, change or drop objects', () => {
    expect(changesStructure('create table t (id int)')).toBe(true)
    expect(changesStructure('SELECT 1;\nALTER TABLE t ADD c int')).toBe(true)
    expect(changesStructure('select created_at from dropped_items')).toBe(false)
  })
})
