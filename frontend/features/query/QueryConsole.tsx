import { useRef } from 'react'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
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
}

/** A SQL editor with the results of its last run below. */
export function QueryConsole({ sessionId, connectionId, database }: QueryConsoleProps) {
  const editor = useRef<SqlEditorHandle>(null)
  const { running, cancelling, cancelError, lastRun, run, cancel } = useQueryRunner(
    sessionId,
    connectionId,
    database,
  )

  return (
    <div className={styles.console}>
      <div className={styles.toolbar}>
        <span className="muted">
          <Icon name="database" size={14} className={styles.inline} /> {database ?? 'default database'}
        </span>
        <span className={styles.spacer} />
        {cancelError && <span className={styles.cancelError}>Could not cancel: {cancelError}</span>}
        {running ? (
          <Button disabled={cancelling} onClick={cancel}>
            <Icon name="stop" />
            {cancelling ? 'Cancelling…' : 'Cancel'}
          </Button>
        ) : (
          <Button variant="primary" onClick={() => run(editor.current?.textToRun() ?? '')}>
            <Icon name="play" />
            Run
            <span className={styles.shortcut}>⌘↵</span>
          </Button>
        )}
      </div>

      <div className={styles.editor}>
        <SqlEditor ref={editor} onRun={run} />
      </div>

      <div className={styles.results} aria-busy={running}>
        {lastRun && 'error' in lastRun ? (
          <p className={styles.error}>{lastRun.error}</p>
        ) : lastRun ? (
          // Remount per run so the selected result tab resets.
          <ResultsPanel key={lastRun.id} outcome={lastRun.outcome} />
        ) : (
          <p className={styles.placeholder}>
            {running
              ? 'Running…'
              : 'Write a query and press ⌘↵ to run it. Select text to run only that part.'}
          </p>
        )}
      </div>
    </div>
  )
}
