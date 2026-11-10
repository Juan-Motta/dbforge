import { useEffect, useState } from 'react'
import { statementLogApi, type StatementEntry } from './api'
import { merge } from './statementLog'

/**
 * Every statement recorded since the app started, kept current as new ones
 * are recorded. Listens from the start, even while the console is hidden, so
 * opening it shows what already ran.
 */
export function useStatementLog() {
  const [entries, setEntries] = useState<StatementEntry[]>([])

  useEffect(() => {
    let current = true
    // Listening starts before the list is read, so nothing recorded in between is missed.
    const stop = statementLogApi.onLogged((entry) => current && setEntries((state) => merge(state, [entry])))
    stop
      .then(() => statementLogApi.list())
      .then((recorded) => current && setEntries((state) => merge(state, recorded)))
      .catch(() => {})
    return () => {
      current = false
      void stop.then((unlisten) => unlisten())
    }
  }, [])

  return {
    entries,
    clear: () => {
      setEntries([])
      void statementLogApi.clear().catch(() => {})
    },
  }
}
