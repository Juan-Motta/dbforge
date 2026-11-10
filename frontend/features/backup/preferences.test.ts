import { describe, expect, it } from 'vitest'
import { backupFileName, knownPreferences } from './preferences'

describe('backup preferences', () => {
  it('keeps only values this version understands', () => {
    expect(knownPreferences({ theme: 'light', language: 'es', exportFolder: ' /backups ' })).toEqual({
      theme: 'light',
      language: 'es',
      exportFolder: '/backups',
    })
    expect(knownPreferences({ theme: 'sepia', language: 'fr', exportFolder: '  ' })).toEqual({
      theme: undefined,
      language: undefined,
      exportFolder: undefined,
    })
  })

  it('names the file after the day', () => {
    expect(backupFileName(new Date(2026, 10, 9))).toBe('dbforge-backup-2026-11-09.json')
  })
})
