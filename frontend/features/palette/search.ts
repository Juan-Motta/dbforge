/** Where the query was found in a label, and how good a hit it is (lower is better). */
export interface Match {
  start: number
  length: number
  score: number
}

/**
 * Case-insensitive substring match. An exact name beats a prefix, a prefix
 * beats the start of a word (`order` in `sales_order`), which beats any hit.
 */
export function matchLabel(label: string, query: string): Match | null {
  if (!query) return { start: 0, length: 0, score: 0 }
  const start = label.toLowerCase().indexOf(query.toLowerCase())
  if (start < 0) return null
  const score =
    query.length === label.length ? 0 : start === 0 ? 1 : /[\s_.\-:]/.test(label[start - 1]) ? 2 : 3
  return { start, length: query.length, score }
}

/** Splits a label around its match, for highlighting. */
export function splitAtMatch(label: string, match: Match): [string, string, string] {
  const end = match.start + match.length
  return [label.slice(0, match.start), label.slice(match.start, end), label.slice(end)]
}

/** A leading `>` limits the search to commands, as in most editors. */
export function parseQuery(raw: string): { text: string; commandsOnly: boolean } {
  const trimmed = raw.trimStart()
  return trimmed.startsWith('>')
    ? { text: trimmed.slice(1).trim(), commandsOnly: true }
    : { text: raw.trim(), commandsOnly: false }
}

/** The items whose label matches, best first, then shortest, then alphabetical. */
export function rank<T extends { label: string }>(
  items: T[],
  query: string,
  limit = Infinity,
): { item: T; match: Match }[] {
  return items
    .flatMap((item) => {
      const match = matchLabel(item.label, query)
      return match ? [{ item, match }] : []
    })
    .sort(
      (a, b) =>
        a.match.score - b.match.score ||
        a.item.label.length - b.item.label.length ||
        a.item.label.localeCompare(b.item.label),
    )
    .slice(0, limit)
}
