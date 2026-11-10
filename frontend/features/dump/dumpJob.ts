import { useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { dumpApi } from './api'

/** What the box under a dump form says once a run is over, or before one starts. */
export type Status =
  | { state: 'idle' }
  | { state: 'done' | 'warning' | 'failed' | 'stopped'; title: string; detail?: string; mono?: boolean }

/** A pg_dump or psql run: its job id while it runs, when it started, and the last line it wrote. */
export function useDumpJob() {
  const [job, setJob] = useState<{ id: string; startedAt: number } | null>(null)
  const [line, setLine] = useState('')

  const run = async <T>(task: (jobId: string) => Promise<T>): Promise<T> => {
    const id = crypto.randomUUID()
    setJob({ id, startedAt: Date.now() })
    setLine('')
    const stopListening = await dumpApi.onProgress(id, setLine)
    try {
      return await task(id)
    } finally {
      stopListening()
      setJob(null)
    }
  }

  const cancel = () => {
    if (job) void dumpApi.cancel(job.id)
  }

  return { running: job !== null, startedAt: job?.startedAt ?? 0, line, run, cancel }
}

/** How a run that threw ended: stopped by the user, or failed with the tool's message. */
export function endedStatus(error: unknown, stopped: string, failed: string): Status {
  const text = errorMessage(error)
  // The backend's word for a run the Stop button ended (tools::cancelled).
  if (text === 'Cancelled.') return { state: 'stopped', title: stopped }
  return { state: 'failed', title: failed, detail: text, mono: true }
}
