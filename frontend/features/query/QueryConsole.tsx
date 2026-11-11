import { useCallback, useEffect, useRef, useState } from 'react'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import type { SqlNames } from '../../components/code/sqlLanguage'
import { isRelationalDialect, type SqlDialect } from '../../domain/connection'
import { useMessages } from '../../lib/i18n/useLanguage'
import { explorerApi } from '../explorer/api'
import type { HistoryEntry } from '../history/api'
import { HistoryPanel } from '../history/HistoryPanel'
import { queryApi, type StatementResult } from './api'
import { changesStructure, completionNames } from './completion'
import { PlanView } from './PlanView'
import { ResultsPanel } from './ResultsPanel'
import { messages } from './messages'
import { SqlEditor, type SqlEditorHandle } from './SqlEditor'
import { useQueryRunner } from './useQueryRunner'
import styles from './QueryConsole.module.css'

interface QueryConsoleProps {
  /** Identifies the console's session, the connection kept open between runs. */
  sessionId: string
  connectionId: string
  /** Runs against this database; the connection's default when omitted. */
  database?: string
  /** Cosmos DB: the container picked first; the console's picker can change it. */
  container?: string
  /** Text the editor starts with, e.g. a query reopened from the history. */
  initialSql?: string
  onExportResult: (result: StatementResult) => void
  dialect: SqlDialect
  /** Opens a query from the history in a new console on its own connection. */
  onOpenQuery: (entry: HistoryEntry) => void
  /** False once a history entry's connection has been deleted. */
  canOpen: (entry: HistoryEntry) => boolean
}

/** A SQL editor with the results of its last run below. */
export function QueryConsole({
  sessionId,
  connectionId,
  database,
  container: firstContainer,
  initialSql,
  onExportResult,
  dialect,
  onOpenQuery,
  canOpen,
}: QueryConsoleProps) {
  const m = useMessages(messages)
  const editor = useRef<SqlEditorHandle>(null)
  const containers = useContainers(dialect === 'cosmosdb' ? connectionId : null, database)
  const [picked, setPicked] = useState(firstContainer)
  // Until one is picked, the database's first container.
  const container = picked ?? containers?.[0]
  const { running, explaining, cancelling, cancelError, lastRun, run, explain, cancel } = useQueryRunner(
    sessionId,
    connectionId,
    { database, container },
  )
  const { names, reload: reloadNames } = useCompletionNames(
    isRelationalDialect(dialect) ? connectionId : null,
    database,
    dialect,
  )
  /** Runs `sql`; a script that may have changed the tables reads their names again. */
  const runScript = (sql: string) =>
    void run(sql).then(() => {
      if (changesStructure(sql)) reloadNames()
    })
  const [analyze, setAnalyze] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const textToRun = () => editor.current?.textToRun() ?? ''

  return (
    <div
      className={styles.console}
      onKeyDown={(event) => {
        // ⌘Y shows or hides the history while focus is in this console.
        if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'y') {
          event.preventDefault()
          setHistoryOpen((open) => !open)
        }
      }}
    >
      <div className={styles.toolbar}>
        <span className="muted">
          <Icon name="database" size={14} className={styles.inline} /> {database ?? m.defaultDatabase}
        </span>
        {containers && (
          <select
            className={styles.container}
            aria-label={m.containerToQuery}
            value={container ?? ''}
            disabled={running || explaining || containers.length === 0}
            onChange={(event) => setPicked(event.target.value)}
          >
            {containers.length === 0 && <option value="">{m.noContainers}</option>}
            {containers.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        )}
        <span className={styles.spacer} />
        {cancelError && <span className={styles.cancelError}>{m.couldNotCancel(cancelError)}</span>}
        <Button
          aria-pressed={historyOpen}
          className={historyOpen ? styles.toggled : undefined}
          title={m.historyTitle}
          onClick={() => setHistoryOpen((open) => !open)}
        >
          <Icon name="history" />
          {m.history}
        </Button>
        <label className={styles.analyze} title={m.analyzeTitle}>
          <input type="checkbox" checked={analyze} onChange={(event) => setAnalyze(event.target.checked)} />
          {m.analyze}
        </label>
        <Button disabled={running || explaining} onClick={() => explain(textToRun(), analyze)}>
          <Icon name="plan" />
          {explaining ? m.explaining : m.explain}
        </Button>
        {running ? (
          <Button disabled={cancelling} onClick={cancel}>
            <Icon name="stop" />
            {cancelling ? m.cancelling : m.cancel}
          </Button>
        ) : (
          <Button variant="primary" disabled={explaining} onClick={() => runScript(textToRun())}>
            <Icon name="play" />
            {m.run}
            <span className={styles.shortcut}>⌘↵</span>
          </Button>
        )}
      </div>

      <div className={styles.body}>
        <div className={styles.main}>
          <div className={styles.editor}>
            <SqlEditor
              ref={editor}
              initialValue={initialSql}
              dialect={dialect}
              names={names}
              onRun={runScript}
            />
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
              <p className={styles.placeholder}>{running || explaining ? m.running : m.placeholder}</p>
            )}
          </div>
        </div>
        {historyOpen && (
          <HistoryPanel
            connectionId={connectionId}
            reloadKey={lastRun?.id}
            canOpen={canOpen}
            onLoad={(entry) => editor.current?.replace(entry.sql)}
            onOpenNew={onOpenQuery}
            onClose={() => setHistoryOpen(false)}
          />
        )}
      </div>
    </div>
  )
}

/**
 * The containers a Cosmos DB console can query, for its picker; `null` for other
 * engines, and while they load. Without a database there is nothing to list.
 */
/**
 * The schemas, tables and columns the editor completes, read when the console
 * opens and again on `reload`. `null` connection: an engine without SQL names.
 * A failure leaves keyword completion alone; it is no reason to stop typing.
 */
function useCompletionNames(connectionId: string | null, database: string | undefined, dialect: SqlDialect) {
  const [names, setNames] = useState<SqlNames | undefined>(undefined)
  const [reloads, setReloads] = useState(0)
  useEffect(() => {
    if (connectionId === null) return
    let current = true
    queryApi.completionSchema(connectionId, database).then(
      (schema) => current && setNames(completionNames(schema, dialect)),
      () => {},
    )
    return () => {
      current = false
    }
  }, [connectionId, database, dialect, reloads])
  const reload = useCallback(() => setReloads((n) => n + 1), [])
  return { names, reload }
}

function useContainers(connectionId: string | null, database: string | undefined): string[] | null {
  const [containers, setContainers] = useState<string[] | null>(null)
  useEffect(() => {
    if (connectionId === null || !database) return
    let current = true
    explorerApi
      .listObjects(connectionId, [database])
      .then((objects) => current && setContainers(objects.map((object) => object.name)))
      .catch(() => current && setContainers([]))
    return () => {
      current = false
    }
  }, [connectionId, database])
  if (connectionId === null) return null
  return database ? containers : []
}
