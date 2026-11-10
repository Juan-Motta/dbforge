import { describe, expect, it } from 'vitest'
import { filtersByValue, narrowedBy, rowAsInsert, rowAsJson, sortedBy, valueCondition } from './cellActions'

describe('cell actions', () => {
  it('write conditions on a value in each dialect', () => {
    expect(valueCondition('postgres', 'name', 'equal', "O'Brien")).toBe(`"name" = 'O''Brien'`)
    expect(valueCondition('mysql', 'name', 'notEqual', 'a\\b')).toBe("`name` <> 'a\\\\b'")
    expect(valueCondition('sqlserver', 'name', 'equal', null)).toBe('[name] IS NULL')
    expect(valueCondition('postgres', 'name', 'notEqual', null)).toBe('"name" IS NOT NULL')
    expect(valueCondition('cosmosdb', 'name', 'isNull', 'x')).toBe('IS_NULL(c.name)')
    expect(valueCondition('cosmosdb', 'total', 'equal', '12')).toBe("c.total = '12'")
    expect(filtersByValue('mongodb')).toBe(false)
  })

  it('narrow an existing filter and sort by one column', () => {
    expect(narrowedBy({ filter: '', order: 'x' }, 'a = 1')).toEqual({ filter: 'a = 1', order: 'x' })
    expect(narrowedBy({ filter: 'b = 2 OR c = 3', order: '' }, 'a = 1').filter).toBe(
      '(b = 2 OR c = 3) AND a = 1',
    )
    expect(sortedBy('postgres', 'name', 'desc', { filter: 'a', order: 'b' })).toEqual({
      filter: 'a',
      order: '"name" DESC',
    })
    expect(sortedBy('mongodb', 'name', 'asc', { filter: '', order: '' }).order).toBe('{name: 1}')
  })

  it('copy a row as JSON or as an INSERT', () => {
    expect(rowAsJson(['id', 'name'], ['1', null])).toBe('{\n  "id": "1",\n  "name": null\n}')
    const users = { database: 'shop', schema: 'public', name: 'users' }
    expect(rowAsInsert('postgres', users, ['id', 'name'], ['1', "O'Brien"])).toBe(
      `INSERT INTO "public"."users" ("id", "name")\nVALUES ('1', 'O''Brien');`,
    )
  })
})
