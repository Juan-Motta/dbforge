import { Callout, CalloutSpace } from '../../components/Callout'
import { useElapsed } from '../../components/useElapsed'
import type { Status } from './dumpJob'

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
