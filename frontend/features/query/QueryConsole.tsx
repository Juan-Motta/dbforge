import { useRef } from 'react'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { ResultsPanel } from './ResultsPanel'
import { SqlEditor, type SqlEditorHandle } from './SqlEditor'
import { useQueryRunner } from './useQueryRunner'
import styles from './QueryConsole.module.css'

interface QueryConsoleProps {
  connectionId: string
  /** Runs against this database; the connection's default when omitted. */
  database?: string
}

/** A SQL editor with the results of its last run below. */
export function QueryConsole({ connectionId, database }: QueryConsoleProps) {
  const editor = useRef<SqlEditorHandle>(null)
  const { running, lastRun, run } = useQueryRunner(connectionId, database)

  return (
    <div className={styles.console}>
      <div className={styles.toolbar}>
        <span className="muted">
          <Icon name="database" size={14} className={styles.inline} /> {database ?? 'default database'}
        </span>
        <span className={styles.spacer} />
        <Button variant="primary" disabled={running} onClick={() => run(editor.current?.textToRun() ?? '')}>
          <Icon name="play" />
          {running ? 'Running…' : 'Run'}
          <span className={styles.shortcut}>⌘↵</span>
        </Button>
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
