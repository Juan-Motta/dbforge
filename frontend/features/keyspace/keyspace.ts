import type { KeyInfo } from './api'

export type KeyNode =
  | { kind: 'folder'; label: string; prefix: string; count: number; children: KeyNode[] }
  | { kind: 'key'; label: string; key: KeyInfo }

/**
 * Groups keys into folders by their `:`-separated prefixes, as most Redis apps
 * name them (`session:7f3a:roles`). Folders come first; each counts the keys under it.
 */
export function keyTree(keys: KeyInfo[], prefix = ''): KeyNode[] {
  const folders = new Map<string, KeyInfo[]>()
  const leaves: KeyNode[] = []
  for (const key of keys) {
    const rest = key.name.slice(prefix.length)
    const cut = rest.indexOf(':')
    if (cut < 0) {
      leaves.push({ kind: 'key', label: key.name, key })
    } else {
      const folder = prefix + rest.slice(0, cut + 1)
      folders.set(folder, [...(folders.get(folder) ?? []), key])
    }
  }
  const byLabel = (a: KeyNode, b: KeyNode) => (a.label < b.label ? -1 : a.label > b.label ? 1 : 0)
  const folderNodes: KeyNode[] = [...folders].map(([folder, inside]) => ({
    kind: 'folder',
    label: folder,
    prefix: folder,
    count: inside.length,
    children: keyTree(inside, folder),
  }))
  return [...folderNodes.sort(byLabel), ...leaves.sort(byLabel)]
}

export interface KeyRow {
  node: KeyNode
  depth: number
}

/** The rows on screen: folders, and the children of expanded ones. */
export function visibleKeyRows(
  nodes: KeyNode[],
  isExpanded: (prefix: string) => boolean,
  depth = 0,
): KeyRow[] {
  return nodes.flatMap((node) => {
    const row = { node, depth }
    if (node.kind === 'key' || !isExpanded(node.prefix)) return [row]
    return [row, ...visibleKeyRows(node.children, isExpanded, depth + 1)]
  })
}

const UNITS: [number, string][] = [
  [86_400_000, 'd'],
  [3_600_000, 'h'],
  [60_000, 'm'],
  [1_000, 's'],
]

/** `23m`, or with `precise` `23m 41s`; `no ttl` for keys that never expire. */
export function formatTtl(ms: number | null, precise = false): string {
  if (ms === null) return 'no ttl'
  if (ms < 1_000) return `${ms}ms`
  const index = UNITS.findIndex(([size]) => ms >= size)
  const [size, unit] = UNITS[index]
  const whole = `${Math.floor(ms / size)}${unit}`
  const next = UNITS[index + 1]
  if (!precise || !next) return whole
  const rest = Math.floor((ms % size) / next[0])
  return rest > 0 ? `${whole} ${rest}${next[1]}` : whole
}

export function formatBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  const digits = unit === 0 || value >= 100 || Number.isInteger(value) ? 0 : 1
  return `${value.toFixed(digits)} ${units[unit]}`
}

export function hitRate(hits: number, misses: number): string {
  const total = hits + misses
  return total === 0 ? '—' : `${((hits / total) * 100).toFixed(1)}%`
}

/** The text pretty-printed when it is a JSON object or array, else `null`. */
export function asJson(text: string): string | null {
  const trimmed = text.trim()
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return null
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2)
  } catch {
    return null
  }
}

/** The text's UTF-8 bytes in hex, space-separated. */
export function asHex(text: string): string {
  return [...new TextEncoder().encode(text)].map((b) => b.toString(16).padStart(2, '0')).join(' ')
}

/** The label the browser shows for a type: `HASH`, `ZSET`, … */
export function kindLabel(kind: string): string {
  return kind.toUpperCase()
}

export function parseScore(text: string): number {
  const score = Number(text.trim())
  if (text.trim() === '' || !Number.isFinite(score)) throw `"${text}" isn't a number.`
  return score
}

/** `field=value` lines; the value may contain `=`. */
export function parseFields(text: string): [string, string][] {
  return text
    .split('\n')
    .filter((line) => line.trim() !== '')
    .map((line) => {
      const cut = line.indexOf('=')
      if (cut <= 0) throw `"${line}" isn't field=value.`
      return [line.slice(0, cut).trim(), line.slice(cut + 1)]
    })
}
