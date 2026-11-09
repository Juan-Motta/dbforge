/**
 * Colors for a document shown as text, JSON or mongosh's literals
 * (`ObjectId("…")`, `NumberLong(5)`), without an editor per document: a page
 * can hold hundreds of them.
 */

export type TokenKind = 'key' | 'string' | 'number' | 'literal' | 'call' | 'plain'

export interface Token {
  text: string
  kind: TokenKind
}

const PATTERN =
  /("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')(\s*:)?|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b)|\b(true|false|null|undefined)\b|\b([A-Za-z_$][\w$]*)(?=\()/g

/** The document split into runs of one kind; joined back they are the text. */
export function documentTokens(text: string): Token[] {
  const tokens: Token[] = []
  const plain = (to: number, from: number) => {
    if (to > from) tokens.push({ text: text.slice(from, to), kind: 'plain' })
  }
  let last = 0
  for (const match of text.matchAll(PATTERN)) {
    const at = match.index
    plain(at, last)
    const [whole, quoted, colon, number, literal, call] = match
    if (quoted !== undefined) {
      tokens.push({ text: quoted, kind: colon ? 'key' : 'string' })
      if (colon) tokens.push({ text: colon, kind: 'plain' })
    } else if (number !== undefined) tokens.push({ text: number, kind: 'number' })
    else if (literal !== undefined) tokens.push({ text: literal, kind: 'literal' })
    else if (call !== undefined) tokens.push({ text: call, kind: 'call' })
    last = at + whole.length
  }
  plain(text.length, last)
  return tokens
}
