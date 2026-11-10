import { describe, expect, it } from 'vitest'
import { messages } from './messages'
import { filterLabels, NO_FILTER, sortBy, sortOf } from './rowFilter'

describe('sortBy', () => {
  it('cycles a column through ascending, descending and no order, quoted for the dialect', () => {
    const asc = sortBy('postgres', 'total', NO_FILTER)
    expect(asc.order).toBe('"total"')
    const desc = sortBy('postgres', 'total', asc)
    expect(desc.order).toBe('"total" DESC')
    expect(sortBy('postgres', 'total', desc).order).toBe('')
    expect(sortBy('mysql', 'we`ird', NO_FILTER).order).toBe('`we``ird`')
    expect(sortBy('sqlserver', 'a]b', NO_FILTER).order).toBe('[a]]b]')
  })

  it('writes MongoDB sorts as documents and keeps the filter', () => {
    const filtered = { filter: "{status: 'paid'}", order: '' }
    expect(sortBy('mongodb', 'total', filtered)).toEqual({ filter: "{status: 'paid'}", order: '{total: 1}' })
    expect(sortBy('mongodb', 'total', { ...filtered, order: '{total: 1}' }).order).toBe('{total: -1}')
    expect(sortBy('mongodb', 'created at', NO_FILTER).order).toBe('{"created at": 1}')
  })

  it('writes Cosmos DB sorts on the item `c`', () => {
    const asc = sortBy('cosmosdb', 'total', NO_FILTER)
    expect(asc.order).toBe('c.total')
    expect(sortBy('cosmosdb', 'total', asc).order).toBe('c.total DESC')
    expect(sortBy('cosmosdb', 'created at', NO_FILTER).order).toBe('c["created at"]')
    expect(sortOf('cosmosdb', 'total', 'c.total DESC')).toBe('desc')
  })

  it('replaces another column’s order', () => {
    expect(sortBy('postgres', 'id', { filter: '', order: 'total DESC, id' }).order).toBe('"id"')
  })
})

describe('sortOf', () => {
  it('tells the header which way a column is sorted', () => {
    expect(sortOf('postgres', 'total', '"total" DESC')).toBe('desc')
    expect(sortOf('postgres', 'total', '"total"')).toBe('asc')
    expect(sortOf('mongodb', 'total', '{total: -1}')).toBe('desc')
    expect(sortOf('postgres', 'total', 'total DESC')).toBeNull()
  })
})

describe('filterLabels', () => {
  it('names MongoDB’s fields in the interface language and keeps SQL keywords', () => {
    expect(filterLabels('mongodb', messages.en)).toMatchObject({ filter: 'Filter', order: 'Sort' })
    expect(filterLabels('mongodb', messages.es)).toMatchObject({ filter: 'Filtro', order: 'Orden' })
    expect(filterLabels('postgres', messages.es)).toMatchObject({ filter: 'WHERE', order: 'ORDER BY' })
  })
})
