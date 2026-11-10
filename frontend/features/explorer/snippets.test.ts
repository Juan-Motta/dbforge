import { describe, expect, it } from 'vitest'
import { insertStatement, previewQuery, qualifiedName, selectStatement } from './snippets'

const orders = { database: 'shop', schema: 'sales', name: 'order "items"' }

describe('snippets', () => {
  it('names a table the way each dialect quotes it', () => {
    expect(qualifiedName('postgres', orders)).toBe('"sales"."order ""items"""')
    expect(qualifiedName('sqlserver', { ...orders, name: 'a]b' })).toBe('[sales].[a]]b]')
    expect(qualifiedName('mysql', { database: 'shop', schema: 'shop', name: 'a`b' })).toBe('`a``b`')
    expect(qualifiedName('sqlite', { database: 'main', schema: 'main', name: 'users' })).toBe('"users"')
  })

  it('asks each engine for its first hundred rows', () => {
    const users = { database: 'shop', schema: 'public', name: 'users' }
    expect(previewQuery('postgres', users)).toBe('SELECT * FROM "public"."users" LIMIT 100;')
    expect(previewQuery('sqlserver', { ...users, schema: 'dbo' })).toBe(
      'SELECT TOP 100 * FROM [dbo].[users];',
    )
    expect(previewQuery('mongodb', { ...users, name: 'my-orders' })).toBe(
      'db.getCollection("my-orders").find({}).limit(100)',
    )
    expect(previewQuery('cosmosdb', users)).toBe('SELECT TOP 100 * FROM c')
  })

  it('writes SELECT and INSERT with every column', () => {
    const users = { database: 'shop', schema: 'public', name: 'users' }
    expect(selectStatement('postgres', users, ['id', 'full name'])).toBe(
      'SELECT\n  "id",\n  "full name"\nFROM "public"."users";',
    )
    expect(insertStatement('mysql', { ...users, schema: 'shop' }, ['id', 'name'])).toBe(
      'INSERT INTO `users` (`id`, `name`)\nVALUES (NULL, NULL);',
    )
  })
})
