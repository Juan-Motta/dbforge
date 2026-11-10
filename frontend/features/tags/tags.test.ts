import { describe, expect, it } from 'vitest'
import { confirmingTag, newTag, tagOf, type Tag } from './tags'

const prod: Tag = { id: 'production', name: 'PROD', color: 'red', confirm: true }
const dev: Tag = { id: 'development', name: 'DEV', color: 'blue', confirm: false }

describe('tags', () => {
  it('finds a connection’s tag, and asks before changes only where the tag says so', () => {
    expect(tagOf([prod, dev], 'development')).toBe(dev)
    expect(tagOf([prod, dev], 'deleted')).toBeNull()
    expect(tagOf([prod, dev], null)).toBeNull()
    expect(confirmingTag([prod, dev], 'production')).toBe('PROD')
    expect(confirmingTag([prod, dev], 'development')).toBeNull()
    expect(confirmingTag([prod, dev], null)).toBeNull()
  })

  it('gives a new tag a color no other tag uses', () => {
    expect(newTag([prod, dev], 'UAT', 'x')).toEqual({ id: 'x', name: 'UAT', color: 'orange', confirm: false })
  })
})
