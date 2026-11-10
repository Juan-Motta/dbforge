/** Where the highlight goes from `index` after `delta` steps, wrapping around `count` options. */
export function step(index: number, delta: number, count: number): number {
  if (count === 0) return -1
  if (index < 0) return delta > 0 ? 0 : count - 1
  return (((index + delta) % count) + count) % count
}

/** The first option at or after `from` (wrapping) whose label starts with `typed`, ignoring case; -1 when none. */
export function matchTyped(labels: string[], typed: string, from: number): number {
  const prefix = typed.toLowerCase()
  for (let offset = 0; offset < labels.length; offset++) {
    const index = (from + offset) % labels.length
    if (labels[index].toLowerCase().startsWith(prefix)) return index
  }
  return -1
}
