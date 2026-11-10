import { useEffect, useState } from 'react'
import { Button } from '../../components/Button'
import { Icon, type IconName } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { TextField } from '../../components/TextField'
import { useMessages } from '../../lib/i18n/useLanguage'
import { explorerApi } from '../explorer/api'
import { formatMs } from '../query/plan'
import { formatBytes } from '../table-structure/format'
import { chooseFolder, chooseSqlFile, dumpApi, type DumpContent, type DumpOptions } from './api'
import { exportFileName, joinPath } from './exportFolder'
import { ToolRow } from './ToolRow'
import { useExportFolder } from './useExportFolder'
import { contentsOf, targetNote } from './dump'
import { DumpStatus } from './DumpStatus'
import { endedStatus, useDumpJob, type Status } from './dumpJob'
import { messages } from './messages'
import styles from './DumpDialogs.module.css'

interface ExportDatabaseDialogProps {
  connectionId: string
  connectionName: string
  database: string
  onClose: () => void
}

const CONTENT_ICONS: Record<DumpContent, IconName> = { all: 'database', structure: 'schema', data: 'table' }

/** Writes a whole database to a .sql file with pg_dump, in the export folder. */
export function ExportDatabaseDialog({
  connectionId,
  connectionName,
  database,
  onClose,
}: ExportDatabaseDialogProps) {
  const m = useMessages(messages)
  const [options, setOptions] = useState<DumpOptions>({ content: 'all', clean: false, keepOwners: false })
  const [status, setStatus] = useState<Status>({ state: 'idle' })
  const exportFolder = useExportFolder()
  // A folder picked for this export only; the settings keep the default.
  const [folder, setFolder] = useState<string | null>(null)
  const [fileName, setFileName] = useState(() => exportFileName(database, new Date()))
  // A file already at the path: the first press says so, the second replaces it.
  const [replacing, setReplacing] = useState(false)
  const job = useDumpJob()
  const target = folder ?? exportFolder.folder
  const path = target && fileName.trim() ? joinPath(target, fileName.trim()) : null

  const start = async () => {
    if (!path) return
    if (!replacing && (await dumpApi.fileExists(path))) {
      setReplacing(true)
      return
    }
    setReplacing(false)
    setStatus({ state: 'idle' })
    try {
      const done = await job.run((jobId) =>
        dumpApi.exportDatabase({ jobId, connectionId, database, path, options }),
      )
      setStatus({
        state: 'done',
        title: m.exported(database, formatBytes(done.bytes), formatMs(done.elapsedMs)),
        detail: path,
        mono: true,
      })
    } catch (error) {
      setStatus(endedStatus(error, m.exportStopped, m.exportFailed))
    }
  }

  const pickFolder = async () => {
    const picked = await chooseFolder(target ?? undefined)
    if (picked) {
      setFolder(picked)
      setReplacing(false)
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
          <strong>{database}</strong>
          <span className="muted">· {connectionName}</span>
        </p>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle} id="dump-content">
            {m.content}
          </h3>
          <div role="radiogroup" aria-labelledby="dump-content" className={styles.list}>
            {contentsOf(m).map((option) => (
              <label key={option.id} className={styles.option}>
                <input
                  type="radio"
                  name="dump-content"
                  checked={options.content === option.id}
                  data-autofocus={options.content === option.id || undefined}
                  disabled={job.running}
                  onChange={() => setOptions({ ...options, content: option.id })}
                />
                <span className={styles.optionIcon}>
                  <Icon name={CONTENT_ICONS[option.id]} size={15} />
                </span>
                <span className={styles.optionText}>
                  <span className={styles.optionTitle}>{option.label}</span>
                  <span className={styles.description}>{option.description}</span>
                </span>
              </label>
            ))}
          </div>
        </section>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>{m.options}</h3>
          <div className={styles.list}>
            <label className={styles.option}>
              <input
                type="checkbox"
                checked={options.clean && options.content !== 'data'}
                disabled={job.running || options.content === 'data'}
                onChange={(event) => setOptions({ ...options, clean: event.target.checked })}
              />
              <span className={styles.optionText}>
                <span className={styles.optionTitle}>{m.clean}</span>
                <span className={styles.description}>{m.cleanDescription}</span>
              </span>
            </label>
            <label className={styles.option}>
              <input
                type="checkbox"
                checked={options.keepOwners}
                disabled={job.running}
                onChange={(event) => setOptions({ ...options, keepOwners: event.target.checked })}
              />
              <span className={styles.optionText}>
                <span className={styles.optionTitle}>{m.keepOwners}</span>
                <span className={styles.description}>{m.keepOwnersDescription}</span>
              </span>
            </label>
          </div>
        </section>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>{m.saveTo}</h3>
          <div className={styles.folderRow}>
            <Icon name="folder" size={15} />
            <span className={styles.path} title={target ?? undefined}>
              <bdi>{target ?? m.lookingForFolder}</bdi>
            </span>
            <Button disabled={job.running} onClick={() => void pickFolder()}>
              {m.change}
            </Button>
          </div>
          <TextField
            label={m.fileName}
            mono
            value={fileName}
            disabled={job.running}
            help={m.folderHelp}
            onChange={(event) => {
              setFileName(event.target.value)
              setReplacing(false)
            }}
          />
        </section>

        <ToolRow name="pg_dump" disabled={job.running} />

        <DumpStatus
          running={job.running}
          runningTitle={m.exportingInto(database)}
          line={job.line}
          startedAt={job.startedAt}
          status={
            replacing
              ? { state: 'warning', title: m.fileExists(fileName.trim()), detail: m.fileExistsDetail }
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
        <Button variant="primary" disabled={job.running || !path} onClick={() => void start()}>
          <Icon name="download" />
          {job.running ? m.exporting : replacing ? m.replace : m.export}
        </Button>
      </footer>
    </Modal>
  )
}

/** The last part of a path, for a result line. */
function fileNameOf(path: string) {
  return path.split(/[\\/]/).pop() ?? path
}

interface ImportDatabaseDialogProps {
  connectionId: string
  connectionName: string
  /** The database the menu was opened on: where the file goes unless another name is typed. */
  database: string
  /** The tag (e.g. PROD) that makes restoring ask to be pressed twice; `null` when it doesn't. */
  confirmTag: string | null
  onClose: () => void
  /** A restore finished: the explorer shows the database, created or changed. */
  onImported: () => void
}

/** Restores a .sql file with psql into a database, creating it when it doesn't exist. */
export function ImportDatabaseDialog({
  connectionId,
  connectionName,
  database,
  confirmTag,
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
    if (confirmTag && !confirming) {
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
        title: done.created ? m.createdAndRestored(name, time) : m.restored(name, time),
        detail: fileNameOf(path),
        mono: true,
      })
      setImports((n) => n + 1)
      onImported()
    } catch (error) {
      setStatus(endedStatus(error, m.importStopped, m.importFailed))
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

        <p className={`${styles.note} ${styles.safety}`}>
          <Icon name="info" size={13} />
          <span>
            {m.importNoteBefore}
            <code>\!</code>
            {m.importNoteAfter}
          </span>
        </p>

        <ToolRow name="psql" disabled={job.running} />

        <DumpStatus
          running={job.running}
          runningTitle={m.importingInto(target.trim())}
          line={job.line}
          startedAt={job.startedAt}
          status={
            confirming
              ? {
                  state: 'warning',
                  title: m.confirmImport(connectionName, confirmTag ?? ''),
                  detail: m.confirmImportDetail(confirmTag ?? ''),
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
          <Icon name="upload" />
          {job.running ? m.importing : confirming ? m.importInto(confirmTag ?? '') : m.import}
        </Button>
      </footer>
    </Modal>
  )
}
