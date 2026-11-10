import { describe, expect, it } from 'vitest'
import { matchTyped, step } from './selectKeys'

describe('select keyboard', () => {
  it('moves the highlight and wraps around', () => {
    expect(step(0, 1, 3)).toBe(1)
    expect(step(2, 1, 3)).toBe(0)
    expect(step(0, -1, 3)).toBe(2)
    expect(step(-1, 1, 3)).toBe(0)
    expect(step(-1, -1, 3)).toBe(2)
    expect(step(0, 1, 0)).toBe(-1)
  })

  it('jumps to the next option starting with what was typed', () => {
    const labels = ['PostgreSQL', 'MySQL', 'MariaDB', 'MongoDB']
    expect(matchTyped(labels, 'm', 0)).toBe(1)
    expect(matchTyped(labels, 'm', 2)).toBe(2)
    expect(matchTyped(labels, 'mo', 0)).toBe(3)
    expect(matchTyped(labels, 'x', 0)).toBe(-1)
  })
})
