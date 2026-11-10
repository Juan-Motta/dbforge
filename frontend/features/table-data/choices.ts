/** One value a cell can take from a list, and how the list shows it. */
export interface Choice {
  value: string
  label: string
}

const TRUE_FALSE: Choice[] = [
  { value: 'true', label: 'true' },
  { value: 'false', label: 'false' },
]

/** SQL Server's `bit` and MySQL's `tinyint(1)` hold a boolean as 1 or 0. */
const ONE_ZERO: Choice[] = [
  { value: '1', label: 'true' },
  { value: '0', label: 'false' },
]

/**
 * The values a column takes from a list: an enum's (from the table structure),
 * or true and false for a boolean, written as the server reads them. `null`
 * for columns that take any text.
 */
export function choicesOf(dataType: string, options?: string[]): Choice[] | null {
  if (options && options.length > 0) return options.map((option) => ({ value: option, label: option }))
  const type = dataType.toLowerCase().trim()
  if (type === 'boolean' || type === 'bool') return TRUE_FALSE
  if (type === 'bit' || type === 'tinyint(1)') return ONE_ZERO
  return null
}
