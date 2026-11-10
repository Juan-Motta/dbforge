import { useEffect, useState } from 'react'
import { statementLogApi, type StatementEntry } from './api'
import { merge } from './statementLog'

/**
 * The statements the backend has recorded, kept current as new ones are
 * recorded. For the console alone, while it is open: the backend keeps the
 * log, so opening it shows what already ran and nothing listens meanwhile.
 * Statements that arrive together (a table opening runs several) are added
 * in one go, once per frame.
 */
export function useStatementLog() {
  const [entries, setEntries] = useState<StatementEntry[]>([])

  useEffect(() => {
    let current = true
    let arrived: StatementEntry[] = []
    let frame = 0
    const add = (entry: StatementEntry) => {
      arrived.push(entry)
      frame ||= requestAnimationFrame(() => {
        const batch = arrived
        arrived = []
        frame = 0
        setEntries((state) => merge(state, batch))
      })
    }
    // Listening starts before the list is read, so nothing recorded in between is missed.
    const stop = statementLogApi.onLogged((entry) => current && add(entry))
    stop
      .then(() => statementLogApi.list())
      .then((recorded) => current && setEntries((state) => merge(state, recorded)))
      .catch(() => {})
    return () => {
      current = false
      cancelAnimationFrame(frame)
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
