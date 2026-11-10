import { useEffect, useState } from 'react'
import { Callout, CalloutSpace } from '../../components/Callout'
import type { Status } from './dumpJob'

/** Whole seconds since `startedAt`, as 0:07 or 1:12, ticking while mounted. */
function useElapsed(startedAt: number) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  const seconds = Math.max(0, Math.floor((now - startedAt) / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}

function Running({ title, line, startedAt }: { title: string; line: string; startedAt: number }) {
  const elapsed = useElapsed(startedAt)
  return <Callout tone="progress" title={title} aside={elapsed} detail={line || ' '} mono oneLine />
}

/**
 * The box under a dump form: the run in progress (with the tool's last line
 * and the time so far), then how it ended. Its space is kept while empty, so
 * the dialog doesn't jump when a run starts or ends.
 */
export function DumpStatus({
  running,
  runningTitle,
  line,
  startedAt,
  status,
}: {
  running: boolean
  runningTitle: string
  line: string
  startedAt: number
  status: Status
}) {
  if (running) return <Running title={runningTitle} line={line} startedAt={startedAt} />
  if (status.state === 'idle') return <CalloutSpace />
  return <Callout tone={status.state} title={status.title} detail={status.detail} mono={status.mono} />
}
