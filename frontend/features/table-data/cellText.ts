/**
 * JSON text laid out again without parsing it into numbers, so big integers and
 * exact decimals stay as the database wrote them. Only whitespace outside strings
 * changes.
 */

/** Whether the text is a JSON object or array. */
export function isJsonDocument(text: string): boolean {
  const trimmed = text.trim()
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return false
  try {
    JSON.parse(trimmed)
    return true
  } catch {
    return false
  }
}

/** Why the text is not valid JSON, or `null` when it is. */
export function jsonError(text: string): string | null {
  try {
    JSON.parse(text)
    return null
  } catch (error) {
    return error instanceof Error ? error.message : String(error)
  }
}

/** The tokens of JSON text: strings whole, every other run of non-space characters split at punctuation. */
function tokens(text: string): string[] {
  const out: string[] = []
  let i = 0
  while (i < text.length) {
    const char = text[i]
    if (char === '"') {
      let end = i + 1
      while (end < text.length && text[end] !== '"') end += text[end] === '\\' ? 2 : 1
      out.push(text.slice(i, end + 1))
      i = end + 1
    } else if ('{}[],:'.includes(char)) {
      out.push(char)
      i++
    } else if (/\s/.test(char)) {
      i++
    } else {
      let end = i
      while (end < text.length && !/[\s{}[\],:"]/.test(text[end])) end++
      out.push(text.slice(i, end))
      i = end
    }
  }
  return out
}

/** JSON text on one line, without spaces. */
export function compactJson(text: string): string {
  return tokens(text).join('')
}

/** JSON text indented by two spaces; empty objects and arrays stay on one line. */
export function prettyJson(text: string): string {
  const list = tokens(text)
  let out = ''
  let depth = 0
  const newline = () => '\n' + '  '.repeat(depth)
  list.forEach((token, i) => {
    const next = list[i + 1]
    if (token === '{' || token === '[') {
      const empty = next === (token === '{' ? '}' : ']')
      out += token
      depth++
      if (!empty) out += newline()
    } else if (token === '}' || token === ']') {
      depth--
      const empty = list[i - 1] === (token === '}' ? '{' : '[')
      out += (empty ? '' : newline()) + token
    } else if (token === ',') {
      out += ',' + newline()
    } else if (token === ':') {
      out += ': '
    } else {
      out += token
    }
  })
  return out
}
