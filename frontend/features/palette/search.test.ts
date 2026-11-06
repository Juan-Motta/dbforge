import { describe, expect, it } from 'vitest'
import { matchLabel, parseQuery, rank, splitAtMatch } from './search'

describe('matchLabel', () => {
  it('ignores case', () => {
    expect(matchLabel('Orders', 'ORD')).toEqual({ start: 0, length: 3, score: 1 })
  })

  it('scores exact, prefix, word start and inner hits in that order', () => {
    expect(matchLabel('orders', 'orders')?.score).toBe(0)
    expect(matchLabel('orders', 'ord')?.score).toBe(1)
    expect(matchLabel('sales_order', 'ord')?.score).toBe(2)
    expect(matchLabel('reorder_level', 'ord')?.score).toBe(3)
  })

  it('returns null without a hit', () => {
    expect(matchLabel('customers', 'ord')).toBeNull()
  })

  it('matches everything for an empty query', () => {
    expect(matchLabel('customers', '')).toEqual({ start: 0, length: 0, score: 0 })
  })
})

describe('splitAtMatch', () => {
  it('splits around the hit', () => {
    expect(splitAtMatch('v_orders_daily', { start: 2, length: 3, score: 2 })).toEqual([
      'v_',
      'ord',
      'ers_daily',
    ])
  })
})

describe('parseQuery', () => {
  it('reads a leading > as commands only', () => {
    expect(parseQuery(' > theme')).toEqual({ text: 'theme', commandsOnly: true })
  })

  it('searches everything otherwise', () => {
    expect(parseQuery(' ord ')).toEqual({ text: 'ord', commandsOnly: false })
  })
})

describe('rank', () => {
  const items = ['reorder_level', 'order_items', 'sales_order', 'orders', 'customers'].map((label) => ({
    label,
  }))

  it('orders by score, then length, then name', () => {
    expect(rank(items, 'order').map(({ item }) => item.label)).toEqual([
      'orders',
      'order_items',
      'sales_order',
      'reorder_level',
    ])
  })

  it('keeps at most `limit` results', () => {
    expect(rank(items, '', 2)).toHaveLength(2)
  })
})
