import { useCallback, useState } from 'react'

/** An on/off choice kept between sessions under `key`; `initial` until one is kept. */
export function useStoredFlag(key: string, initial: boolean) {
  const [flag, setFlag] = useState(() => {
    try {
      const saved = localStorage.getItem(key)
      return saved === null ? initial : saved === 'true'
    } catch {
      return initial
    }
  })
  // Stable, so effects can depend on it.
  const set = useCallback(
    (next: boolean | ((current: boolean) => boolean)) =>
      setFlag((current) => {
        const value = typeof next === 'function' ? next(current) : next
        try {
          localStorage.setItem(key, String(value))
        } catch {
          // Not kept for next time; the choice still applies now.
        }
        return value
      }),
    [key],
  )
  return [flag, set] as const
}
