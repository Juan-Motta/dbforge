import { useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { queryApi, type QueryOutcome } from './api'

/** The latest finished run; `id` changes on every run. */
export type LastRun = { id: number } & ({ outcome: QueryOutcome } | { error: string })

export function useQueryRunner(connectionId: string, database: string | undefined) {
  const [running, setRunning] = useState(false)
  const [lastRun, setLastRun] = useState<LastRun | null>(null)

  const run = async (sql: string) => {
    if (!sql.trim() || running) return
    setRunning(true)
    const id = (lastRun?.id ?? 0) + 1
    try {
      setLastRun({ id, outcome: await queryApi.run(connectionId, database, sql) })
    } catch (error) {
      setLastRun({ id, error: errorMessage(error) })
    } finally {
      setRunning(false)
    }
  }

  return { running, lastRun, run }
}
