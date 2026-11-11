import { describe, expect, it } from 'vitest'
import { CHECK_EVERY, checkDue } from './schedule'

describe('checkDue', () => {
  const now = 1_800_000_000_000

  it('checks the first time, and once a day after', () => {
    expect(checkDue(null, now)).toBe(true)
    expect(checkDue(now - CHECK_EVERY + 1, now)).toBe(false)
    expect(checkDue(now - CHECK_EVERY, now)).toBe(true)
  })

  it('checks when the clock went back past the last check', () => {
    expect(checkDue(now + 60_000, now)).toBe(true)
  })
})
