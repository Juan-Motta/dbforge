import { useRef, useState } from 'react'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import type { SqlDialect } from '../../domain/connection'
import type { StatementResult } from './api'
import { PlanView } from './PlanView'
import { ResultsPanel } from './ResultsPanel'
import { SqlEditor, type SqlEditorHandle } from './SqlEditor'
import { useQueryRunner } from './useQueryRunner'
import styles from './QueryConsole.module.css'

interface QueryConsoleProps {
  /** Identifies the console's session, the connection kept open between runs. */
  sessionId: string
  connectionId: string
  /** Runs against this database; the connection's default when omitted. */
  database?: string
  /** Text the editor starts with, e.g. a query reopened from the history. */
  initialSql?: string
  onExportResult: (result: StatementResult) => void
  dialect: SqlDialect
}

/** A SQL editor with the results of its last run below. */
export function QueryConsole({
  sessionId,
  connectionId,
  database,
  initialSql,
  onExportResult,
  dialect,
}: QueryConsoleProps) {
  const editor = useRef<SqlEditorHandle>(null)
  const { running, explaining, cancelling, cancelError, lastRun, run, explain, cancel } = useQueryRunner(
    sessionId,
    connectionId,
    database,
  )
  const [analyze, setAnalyze] = useState(false)
  const textToRun = () => editor.current?.textToRun() ?? ''

  return (
    <div className={styles.console}>
      <div className={styles.toolbar}>
        <span className="muted">
          <Icon name="database" size={14} className={styles.inline} /> {database ?? 'default database'}
        </span>
        <span className={styles.spacer} />
        {cancelError && <span className={styles.cancelError}>Could not cancel: {cancelError}</span>}
        <label
          className={styles.analyze}
          title="Runs the statement to measure it, inside a transaction that is rolled back"
        >
          <input type="checkbox" checked={analyze} onChange={(event) => setAnalyze(event.target.checked)} />
          Analyze
        </label>
        <Button disabled={running || explaining} onClick={() => explain(textToRun(), analyze)}>
          <Icon name="plan" />
          {explaining ? 'Explaining…' : 'Explain'}
        </Button>
        {running ? (
          <Button disabled={cancelling} onClick={cancel}>
            <Icon name="stop" />
            {cancelling ? 'Cancelling…' : 'Cancel'}
          </Button>
        ) : (
          <Button variant="primary" disabled={explaining} onClick={() => run(textToRun())}>
            <Icon name="play" />
            Run
            <span className={styles.shortcut}>⌘↵</span>
          </Button>
        )}
      </div>

      <div className={styles.editor}>
        <SqlEditor ref={editor} initialValue={initialSql} dialect={dialect} onRun={run} />
      </div>

      <div className={styles.results} aria-busy={running || explaining}>
        {lastRun && 'error' in lastRun ? (
          <p className={styles.error}>{lastRun.error}</p>
        ) : lastRun && 'plan' in lastRun ? (
          <PlanView key={lastRun.id} plan={lastRun.plan} />
        ) : lastRun ? (
          // Remount per run so the selected result tab resets.
          <ResultsPanel key={lastRun.id} outcome={lastRun.outcome} onExport={onExportResult} />
        ) : (
          <p className={styles.placeholder}>
            {running || explaining
              ? 'Running…'
              : 'Write a query and press ⌘↵ to run it. Select text to run only that part.'}
          </p>
        )}
      </div>
    </div>
  )
}
