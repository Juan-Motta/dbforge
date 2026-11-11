import { useLayoutEffect, useRef, useState } from 'react'

type Handlers = Record<string, (...args: never[]) => unknown>

/**
 * `handlers` behind functions that keep their identity across renders and
 * always call the latest version, so memoized children that take them aren't
 * drawn again because a closure changed. Call them from events, not while
 * rendering; the set of names is the one of the first render.
 */
export function useStableHandlers<T extends Handlers>(handlers: T): T {
  const latest = useRef(handlers)
  useLayoutEffect(() => {
    latest.current = handlers
  })
  const [stable] = useState(
    () =>
      Object.fromEntries(
        Object.keys(handlers).map((name) => [
          name,
          (...args: never[]) => (latest.current[name] as (...args: never[]) => unknown)(...args),
        ]),
      ) as T,
  )
  return stable
}
