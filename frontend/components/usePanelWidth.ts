import { useState } from 'react'

/** A panel width the user can drag, kept between sessions under `key`, within `min` and `max`. */
export function usePanelWidth(key: string, initial: number, min: number, max: number) {
  const clamp = (width: number) => Math.min(max, Math.max(min, width))
  const [width, setWidth] = useState(() => {
    try {
      const saved = Number(localStorage.getItem(key))
      return saved > 0 ? clamp(saved) : initial
    } catch {
      return initial
    }
  })
  const set = (next: number) => {
    const value = clamp(next)
    setWidth(value)
    try {
      localStorage.setItem(key, String(value))
    } catch {
      // Not kept for next time; the width still applies now.
    }
  }
  return [width, set, () => set(initial)] as const
}
