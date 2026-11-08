import { describe, expect, it } from 'vitest'
import { linkFilter, referencedTable } from './foreignLinks'

const orders = { database: 'shop', schema: 'public', name: 'orders' }
const fk = (columns: string[], table: string, referencedColumns: string[]) => ({
  name: 'fk',
  columns,
  referencedTable: table,
  referencedColumns,
  onUpdate: 'NO ACTION',
  onDelete: 'NO ACTION',
})

describe('referencedTable', () => {
  it('stays in the table’s schema unless the key names another', () => {
    expect(referencedTable(orders, 'customers')).toEqual({
      database: 'shop',
      schema: 'public',
      name: 'customers',
    })
    expect(referencedTable(orders, 'billing.invoices')).toEqual({
      database: 'shop',
      schema: 'billing',
      name: 'invoices',
    })
    expect(referencedTable(orders, '"Odd.Schema"."My ""T"""')).toEqual({
      database: 'shop',
      schema: 'Odd.Schema',
      name: 'My "T"',
    })
  })
})

describe('linkFilter', () => {
  const row = { customer_id: '877', region: "O'Hara", note: null }

  it('filters the referenced table by the row’s key values, quoted for the dialect', () => {
    expect(linkFilter('postgres', fk(['customer_id'], 'customers', ['id']), row)).toEqual({
      filter: `"id" = '877'`,
      order: '',
    })
    expect(
      linkFilter('mysql', fk(['customer_id', 'region'], 'customers', ['id', 'region']), row)?.filter,
    ).toBe("`id` = '877' AND `region` = 'O''Hara'")
    expect(linkFilter('sqlserver', fk(['customer_id'], 'customers', ['id']), row)?.filter).toBe(
      "[id] = '877'",
    )
  })

  it('has no link while a key column is NULL', () => {
    expect(linkFilter('postgres', fk(['note'], 'notes', ['id']), row)).toBeNull()
  })
})
