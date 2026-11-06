import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { queryApi, type QueryOutcome } from './api'

/** The latest finished run; `id` changes on every run. */
export type LastRun = { id: number } & ({ outcome: QueryOutcome } | { error: string })

/** Runs queries in the console's session and closes the session on unmount. */
export function useQueryRunner(sessionId: string, connectionId: string, database: string | undefined) {
  const [running, setRunning] = useState(false)
  const [cancelling, setCancelling] = useState(false)
  const [cancelError, setCancelError] = useState<string | null>(null)
  const [lastRun, setLastRun] = useState<LastRun | null>(null)

  // The console's tab was closed: release its connection. Nobody is left to show an error.
  useEffect(() => () => void queryApi.closeSession(sessionId).catch(() => {}), [sessionId])

  const run = async (sql: string) => {
    if (!sql.trim() || running) return
    setRunning(true)
    setCancelError(null)
    const id = (lastRun?.id ?? 0) + 1
    try {
      setLastRun({ id, outcome: await queryApi.run(sessionId, connectionId, database, sql) })
    } catch (error) {
      setLastRun({ id, error: errorMessage(error) })
    } finally {
      setRunning(false)
      setCancelling(false)
    }
  }

  const cancel = async () => {
    setCancelling(true)
    try {
      await queryApi.cancel(sessionId)
    } catch (error) {
      // The query keeps running; let the user try again.
      setCancelError(errorMessage(error))
      setCancelling(false)
    }
  }

  return { running, cancelling, cancelError, lastRun, run, cancel }
}
