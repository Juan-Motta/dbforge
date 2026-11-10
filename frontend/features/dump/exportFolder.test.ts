import { describe, expect, it } from 'vitest'
import { defaultExportFolder, exportFileName, joinPath } from './exportFolder'

describe('export folder', () => {
  it('joins a file to a folder with its own separator', () => {
    expect(joinPath('/Users/ann/Exports/', 'shop.sql')).toBe('/Users/ann/Exports/shop.sql')
    expect(joinPath('C:\\Users\\ann\\Exports', 'shop.sql')).toBe('C:\\Users\\ann\\Exports\\shop.sql')
    expect(defaultExportFolder('/Users/ann/Documents')).toBe('/Users/ann/Documents/DBForge')
  })

  it('names an export after its database and the minute, without characters files can’t hold', () => {
    expect(exportFileName('shop', new Date(2026, 10, 10, 14, 32))).toBe('shop-2026-11-10-1432.sql')
    expect(exportFileName('a/b:c', new Date(2026, 0, 2, 3, 4))).toBe('a-b-c-2026-01-02-0304.sql')
  })
})
