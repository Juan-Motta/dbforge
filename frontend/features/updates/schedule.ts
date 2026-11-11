/** How often the automatic check runs: once a day, as ColiUI's updater does. */
export const CHECK_EVERY = 24 * 60 * 60 * 1000

const LAST_CHECK = 'dbforge.updates.lastCheck'

/** Whether an automatic check is due at `now`, the last one having run at `last` (`null`: never). */
export function checkDue(last: number | null, now: number): boolean {
  return last === null || now - last >= CHECK_EVERY || now < last
}

export function lastCheck(): number | null {
  try {
    const saved = Number(localStorage.getItem(LAST_CHECK))
    return Number.isFinite(saved) && saved > 0 ? saved : null
  } catch {
    return null
  }
}

export function noteCheck(at: number) {
  try {
    localStorage.setItem(LAST_CHECK, String(at))
  } catch {
    // Not kept: the next launch checks again, which is harmless.
  }
}
