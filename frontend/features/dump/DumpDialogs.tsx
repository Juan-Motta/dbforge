import { useEffect, useState } from 'react'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { TextField } from '../../components/TextField'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { explorerApi } from '../explorer/api'
import { formatMs } from '../query/plan'
import { formatBytes } from '../table-structure/format'
import { chooseDumpFile, chooseSqlFile, dumpApi, type DumpOptions } from './api'
import { contentsOf, targetNote } from './dump'
import { messages } from './messages'
import styles from './DumpDialogs.module.css'

type Status = { state: 'idle' } | { state: 'done'; message: string } | { state: 'failed'; error: string }

/** A pg_dump or psql run: its job id while it runs, and the last line it wrote. */
function useDumpJob() {
  const [job, setJob] = useState<string | null>(null)
  const [line, setLine] = useState('')

  const run = async <T,>(task: (jobId: string) => Promise<T>): Promise<T> => {
    const jobId = crypto.randomUUID()
    setJob(jobId)
    setLine('')
    const stopListening = await dumpApi.onProgress(jobId, setLine)
    try {
      return await task(jobId)
    } finally {
      stopListening()
      setJob(null)
    }
  }

  const cancel = () => {
    if (job) void dumpApi.cancel(job)
  }

  return { running: job !== null, line, run, cancel }
}

/** One line under the form: what the tool is doing, the result, or what went wrong. */
function StatusLine({ running, line, status }: { running: boolean; line: string; status: Status }) {
  const m = useMessages(messages)
  return (
    <p className={styles.status} role="status">
      {running ? (
        <span className={styles.progress} title={line}>
          {line || m.starting}
        </span>
      ) : status.state === 'done' ? (
        status.message
      ) : status.state === 'failed' ? (
        <span className={styles.error}>{status.error}</span>
      ) : null}
    </p>
  )
}

interface ExportDatabaseDialogProps {
  connectionId: string
  connectionName: string
  database: string
  onClose: () => void
}

/** Writes a whole database to a .sql file with pg_dump. */
export function ExportDatabaseDialog({
  connectionId,
  connectionName,
  database,
  onClose,
}: ExportDatabaseDialogProps) {
  const m = useMessages(messages)
  const [options, setOptions] = useState<DumpOptions>({ content: 'all', clean: false, keepOwners: false })
  const [status, setStatus] = useState<Status>({ state: 'idle' })
  const job = useDumpJob()

  const start = async () => {
    const path = await chooseDumpFile(database)
    if (!path) return
    setStatus({ state: 'idle' })
    try {
      const done = await job.run((jobId) =>
        dumpApi.exportDatabase({ jobId, connectionId, database, path, options }),
      )
      setStatus({
        state: 'done',
        message: m.exported(formatBytes(done.bytes), path, formatMs(done.elapsedMs)),
      })
    } catch (error) {
      setStatus({ state: 'failed', error: errorMessage(error) })
    }
  }

  const close = () => {
    job.cancel()
    onClose()
  }

  return (
    <Modal title={m.exportTitle(database)} onClose={close} className={styles.dialog}>
      <div className={styles.body}>
        <p className={styles.source}>
          <Icon name="database" />
          {database}
          <span className="muted">{m.exportSource(connectionName)}</span>
        </p>

        <div>
          <span className={styles.label} id="dump-content">
            {m.include}
          </span>
          <div role="radiogroup" aria-labelledby="dump-content" className={styles.choices}>
            {contentsOf(m).map((option) => (
              <button
                key={option.id}
                type="button"
                role="radio"
                aria-checked={options.content === option.id}
                data-autofocus={options.content === option.id || undefined}
                disabled={job.running}
                className={styles.choice}
                onClick={() => setOptions({ ...options, content: option.id })}
              >
                <strong>{option.label}</strong>
                <span className={styles.description}>{option.description}</span>
              </button>
            ))}
          </div>
        </div>

        <div className={styles.checks}>
          <label className={styles.check}>
            <input
              type="checkbox"
              checked={options.clean && options.content !== 'data'}
              disabled={job.running || options.content === 'data'}
              onChange={(event) => setOptions({ ...options, clean: event.target.checked })}
            />
            <span>
              {m.clean}
              <span className={styles.description}>{m.cleanDescription}</span>
            </span>
          </label>
          <label className={styles.check}>
            <input
              type="checkbox"
              checked={options.keepOwners}
              disabled={job.running}
              onChange={(event) => setOptions({ ...options, keepOwners: event.target.checked })}
            />
            <span>
              {m.keepOwners}
              <span className={styles.description}>{m.keepOwnersDescription}</span>
            </span>
          </label>
        </div>

        <StatusLine running={job.running} line={job.line} status={status} />
      </div>

      <footer className={styles.footer}>
        {job.running ? (
          <Button onClick={job.cancel}>
            <Icon name="stop" />
            {m.stop}
          </Button>
        ) : (
          <Button onClick={onClose}>{status.state === 'done' ? m.done : m.cancel}</Button>
        )}
        <Button variant="primary" disabled={job.running} onClick={() => void start()}>
          <Icon name="download" />
          {job.running ? m.exporting : m.export}
        </Button>
      </footer>
    </Modal>
  )
}

interface ImportDatabaseDialogProps {
  connectionId: string
  connectionName: string
  /** The database the menu was opened on: where the file goes unless another name is typed. */
  database: string
  /** Restoring on a production connection asks to be pressed twice. */
  production: boolean
  onClose: () => void
  /** A restore finished: the explorer shows the database, created or changed. */
  onImported: () => void
}

/** Restores a .sql file with psql into a database, creating it when it doesn't exist. */
export function ImportDatabaseDialog({
  connectionId,
  connectionName,
  database,
  production,
  onClose,
  onImported,
}: ImportDatabaseDialogProps) {
  const m = useMessages(messages)
  const [path, setPath] = useState<string | null>(null)
  const [target, setTarget] = useState(database)
  const [databases, setDatabases] = useState<string[] | null>(null)
  // Counts finished imports, which may have created a database.
  const [imports, setImports] = useState(0)
  const [status, setStatus] = useState<Status>({ state: 'idle' })
  // On production, the first press only asks for the second.
  const [confirming, setConfirming] = useState(false)
  const job = useDumpJob()

  useEffect(() => {
    let current = true
    explorerApi
      .listObjects(connectionId, [])
      .then((objects) => current && setDatabases(objects.map((object) => object.name)))
      .catch(() => {})
    return () => {
      current = false
    }
  }, [connectionId, imports])

  const choose = async () => {
    const chosen = await chooseSqlFile(m.allFiles)
    if (chosen) {
      setPath(chosen)
      setStatus({ state: 'idle' })
      setConfirming(false)
    }
  }

  const start = async () => {
    if (!path) return
    if (production && !confirming) {
      setConfirming(true)
      return
    }
    setConfirming(false)
    const name = target.trim()
    setStatus({ state: 'idle' })
    try {
      const done = await job.run((jobId) =>
        dumpApi.importDatabase({ jobId, connectionId, database: name, path }),
      )
      const time = formatMs(done.elapsedMs)
      setStatus({
        state: 'done',
        message: done.created ? m.createdAndRestored(name, time) : m.restored(name, time),
      })
      setImports((n) => n + 1)
      onImported()
    } catch (error) {
      setStatus({ state: 'failed', error: errorMessage(error) })
    }
  }

  const close = () => {
    job.cancel()
    onClose()
  }

  return (
    <Modal title={m.importTitle} onClose={close} className={styles.dialog}>
      <div className={styles.body}>
        <div>
          <span className={styles.label}>{m.file}</span>
          <div className={styles.fileRow}>
            <span className={path ? styles.path : `${styles.path} muted`} title={path ?? undefined}>
              {/* Right to left, so a long path shows its end; the path itself stays left to right. */}
              <bdi>{path ?? m.filePlaceholder}</bdi>
            </span>
            <Button data-autofocus disabled={job.running} onClick={() => void choose()}>
              {m.choose}
            </Button>
          </div>
        </div>

        <div>
          <TextField
            label={m.databaseOn(connectionName)}
            mono
            value={target}
            disabled={job.running}
            onChange={(event) => {
              setTarget(event.target.value)
              setStatus({ state: 'idle' })
              setConfirming(false)
            }}
          />
          <p className={styles.note}>{targetNote(target, databases, m)}</p>
        </div>

        <p className={styles.note}>
          {m.importNoteBefore}
          <code>\!</code>
          {m.importNoteAfter}
        </p>

        <StatusLine
          running={job.running}
          line={job.line}
          status={
            confirming
              ? {
                  state: 'failed',
                  error: m.productionConfirm(connectionName),
                }
              : status
          }
        />
      </div>

      <footer className={styles.footer}>
        {job.running ? (
          <Button onClick={job.cancel}>
            <Icon name="stop" />
            {m.stop}
          </Button>
        ) : (
          <Button onClick={onClose}>{status.state === 'done' ? m.done : m.cancel}</Button>
        )}
        <Button
          variant="primary"
          disabled={job.running || !path || target.trim() === ''}
          onClick={() => void start()}
        >
          {job.running ? m.importing : confirming ? m.importIntoProduction : m.import}
        </Button>
      </footer>
    </Modal>
  )
}
