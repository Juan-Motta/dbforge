import { useState } from 'react'

/** A panel size (width or height) the user can drag, kept between sessions under `key`, within `min` and `max`. */
export function usePanelSize(key: string, initial: number, min: number, max: number) {
  const clamp = (size: number) => Math.min(max, Math.max(min, size))
  const [size, setSize] = useState(() => {
    try {
      const saved = Number(localStorage.getItem(key))
      return saved > 0 ? clamp(saved) : initial
    } catch {
      return initial
    }
  })
  const set = (next: number) => {
    const value = clamp(next)
    setSize(value)
    try {
      localStorage.setItem(key, String(value))
    } catch {
      // Not kept for next time; the size still applies now.
    }
  }
  return [size, set, () => set(initial)] as const
}
