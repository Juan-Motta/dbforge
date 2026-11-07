import { describe, expect, it } from 'vitest'
import { encode } from '@msgpack/msgpack'
import {
  asHex,
  asJson,
  asMsgpack,
  formatBytes,
  formatTtl,
  hitRate,
  keyTree,
  visibleKeyRows,
} from './keyspace'
import type { KeyInfo } from './api'

const key = (name: string, kind = 'string', ttlMs: number | null = null): KeyInfo => ({ name, kind, ttlMs })

describe('keyTree', () => {
  it('groups keys by their `:` prefixes, folders first, with key counts', () => {
    const tree = keyTree([
      key('session:b'),
      key('session:a:roles', 'set'),
      key('session:a'),
      key('plain'),
      key('cart:1'),
    ])
    expect(tree.map((n) => [n.kind, n.label, n.kind === 'folder' ? n.count : null])).toEqual([
      ['folder', 'cart:', 1],
      ['folder', 'session:', 3],
      ['key', 'plain', null],
    ])
    const session = tree[1]
    expect(session.kind === 'folder' && session.children.map((n) => n.label)).toEqual([
      'session:a:',
      'session:a',
      'session:b',
    ])
  })

  it('lists collapsed folders as one row and expanded ones with their children', () => {
    const tree = keyTree([key('a:1'), key('a:2'), key('b')])
    expect(visibleKeyRows(tree, () => false).map((r) => [r.depth, r.node.label])).toEqual([
      [0, 'a:'],
      [0, 'b'],
    ])
    expect(visibleKeyRows(tree, (prefix) => prefix === 'a:').map((r) => [r.depth, r.node.label])).toEqual([
      [0, 'a:'],
      [1, 'a:1'],
      [1, 'a:2'],
      [0, 'b'],
    ])
  })
})

describe('formats', () => {
  it('writes TTLs in their largest unit', () => {
    expect(formatTtl(null)).toBe('no ttl')
    expect(formatTtl(41_000)).toBe('41s')
    expect(formatTtl(23 * 60_000 + 41_000)).toBe('23m')
    expect(formatTtl(2 * 3_600_000 + 5)).toBe('2h')
    expect(formatTtl(3 * 86_400_000)).toBe('3d')
    expect(formatTtl(23 * 60_000 + 41_000, true)).toBe('23m 41s')
  })

  it('writes sizes and the hit rate', () => {
    expect(formatBytes(412)).toBe('412 B')
    expect(formatBytes(412 * 1024 * 1024)).toBe('412 MB')
    expect(formatBytes(1536)).toBe('1.5 KB')
    expect(hitRate(974, 26)).toBe('97.4%')
    expect(hitRate(0, 0)).toBe('—')
  })

  it('shows values as JSON or hex when they can be', () => {
    expect(asJson('{"a":[1,2]}')).toBe('{\n  "a": [\n    1,\n    2\n  ]\n}')
    expect(asJson('Mozilla/5.0')).toBeNull()
    expect(asHex('AB')).toBe('41 42')
    expect(asHex('é')).toBe('c3 a9')
  })
})

describe('asMsgpack', () => {
  const hex = (bytes: Uint8Array) => [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('')

  it('decodes binary values the backend sent as hex', () => {
    const packed = encode({
      user: 'u_88412',
      roles: ['admin'],
      seen: 1730718422,
      ok: true,
      blob: new Uint8Array([1, 255]),
    })
    expect(asMsgpack({ text: hex(packed), binary: true })).toBe(
      '{\n  "user": "u_88412",\n  "roles": [\n    "admin"\n  ],\n  "seen": 1730718422,\n  "ok": true,\n  "blob": "0x01ff"\n}',
    )
  })

  it('leaves text alone: MessagePack is binary, and "5" would read as 53', () => {
    expect(asMsgpack({ text: '5', binary: false })).toBeNull()
  })

  it('refuses anything that is not one whole MessagePack value', () => {
    expect(asMsgpack({ text: 'hello', binary: false })).toBeNull()
    expect(asMsgpack({ text: 'ff01', binary: true })).toBeNull()
  })
})
