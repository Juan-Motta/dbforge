import { describe, expect, it } from 'vitest'
import { readNotes } from './notes'

describe('readNotes', () => {
  it('reads the headings and items the release notes script writes', () => {
    const notes =
      '## Features\n\n- **import:** import rows from a file\n- plain item\n\n## Fixes\n\nSee the site.\n'
    expect(readNotes(notes)).toEqual([
      { kind: 'heading', text: 'Features' },
      {
        kind: 'item',
        runs: [
          { text: 'import:', bold: true },
          { text: ' import rows from a file', bold: false },
        ],
      },
      { kind: 'item', runs: [{ text: 'plain item', bold: false }] },
      { kind: 'heading', text: 'Fixes' },
      { kind: 'text', runs: [{ text: 'See the site.', bold: false }] },
    ])
  })

  it('reads no notes as nothing', () => {
    expect(readNotes('')).toEqual([])
    expect(readNotes('\n  \n')).toEqual([])
  })
})
