import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { queryApi, type QueryOutcome, type QueryScope } from './api'
import { messages } from './messages'
import { parsePlan } from './plan'

/**
 * The latest finished run or explain; `id` changes every time. A plan is kept as
 * the server's JSON so it is read again in the interface language.
 */
export type LastRun = { id: number } & ({ outcome: QueryOutcome } | { plan: unknown } | { error: string })

/** Runs queries in the console's session and closes the session on unmount. */
export function useQueryRunner(sessionId: string, connectionId: string, scope: QueryScope) {
  const [running, setRunning] = useState(false)
  const [explaining, setExplaining] = useState(false)
  const [cancelling, setCancelling] = useState(false)
  const [cancelError, setCancelError] = useState<string | null>(null)
  const [lastRun, setLastRun] = useState<LastRun | null>(null)

  // The console's tab was closed: release its connection. Nobody is left to show an error.
  useEffect(() => () => void queryApi.closeSession(sessionId).catch(() => {}), [sessionId])

  const run = async (sql: string) => {
    if (!sql.trim() || running || explaining) return
    setRunning(true)
    setCancelError(null)
    const id = (lastRun?.id ?? 0) + 1
    try {
      setLastRun({ id, outcome: await queryApi.run(sessionId, connectionId, scope, sql) })
    } catch (error) {
      setLastRun({ id, error: errorMessage(error) })
    } finally {
      setRunning(false)
      setCancelling(false)
    }
  }

  /** Shows the statement's plan; it runs only with `analyze`, and is rolled back. */
  const explain = async (sql: string, analyze: boolean) => {
    if (!sql.trim() || running || explaining) return
    setExplaining(true)
    const id = (lastRun?.id ?? 0) + 1
    try {
      const plan = await queryApi.explain(connectionId, scope, sql, analyze)
      // Read once here so a plan that cannot be read shows as an error.
      parsePlan(plan, messages.en)
      setLastRun({ id, plan })
    } catch (error) {
      setLastRun({ id, error: errorMessage(error) })
    } finally {
      setExplaining(false)
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

  return { running, explaining, cancelling, cancelError, lastRun, run, explain, cancel }
}
