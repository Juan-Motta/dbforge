import { useEffect, useRef, useState } from 'react'
import { Button } from '../../components/Button'
import { Callout, CalloutSpace } from '../../components/Callout'
import { Icon, type IconName } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { Option, OptionList } from '../../components/OptionList'
import { useElapsed } from '../../components/useElapsed'
import type { TableRef } from '../../domain/table'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { isMac } from '../../lib/platform'
import { structureApi } from '../table-structure/api'
import { isFiltered, type RowFilter } from '../table-data/rowFilter'
import { chooseFile, exportApi, type RowsSource } from './api'
import { ColumnPicker } from './ColumnPicker'
import { exportOptions, FORMATS, type ExportFormat } from './formats'
import { FormatOptionsFields } from './FormatOptionsFields'
import { messages } from './messages'
import { loadChoice, saveChoice } from './preferences'
import styles from './ExportDialog.module.css'

/** A whole table, streamed from the server, or rows the UI already holds. */
export type ExportSource =
  | { kind: 'table'; connectionId: string; table: TableRef; filter: RowFilter }
  | ({ kind: 'rows' } & RowsSource)

type Status =
  | { state: 'idle' }
  | { state: 'working'; rows: number; startedAt: number }
  | { state: 'saved'; rows: number; path: string }
  | { state: 'copied' }
  | { state: 'stopped' }
  | { state: 'failed'; error: string }

/** A filtered table's rows: those the filter keeps, or every row. */
type Scope = 'filtered' | 'all'

const FORMAT_ICONS: Record<ExportFormat, IconName> = {
  csv: 'table',
  tsv: 'column',
  json: 'code',
  sql: 'database',
  markdown: 'edit',
}

interface ExportDialogProps {
  source: ExportSource
  onClose: () => void
}

export function ExportDialog({ source, onClose }: ExportDialogProps) {
  const m = useMessages(messages)
  const [choice, setChoice] = useState(loadChoice)
  const { format, options } = choice
  const [status, setStatus] = useState<Status>({ state: 'idle' })
  const filtered = source.kind === 'table' && isFiltered(source.filter)
  const [scope, setScope] = useState<Scope>('filtered')
  const columns = useColumns(source)
  const [chosen, setChosen] = useState<string[] | null>(null)
  // Every column until the user leaves some out.
  const chosenColumns = chosen ?? columns ?? []
  // The running table export, and whether the user stopped it.
  const job = useRef<{ id: string; stopped: boolean } | null>(null)
  const name = source.kind === 'table' ? source.table.name : source.name
  const working = status.state === 'working'
  const ready = !working && (columns == null || chosenColumns.length > 0)

  const choose = (change: Partial<typeof choice>) => {
    setChoice({ ...choice, ...change })
    setStatus({ state: 'idle' })
  }

  const sent = () => exportOptions(options, chosenColumns, columns ?? chosenColumns)

  const run = async (task: () => Promise<Status | null>) => {
    setStatus({ state: 'working', rows: 0, startedAt: Date.now() })
    try {
      const done = await task()
      if (done) saveChoice(choice)
      setStatus(done ?? { state: 'idle' })
    } catch (error) {
      setStatus(job.current?.stopped ? { state: 'stopped' } : { state: 'failed', error: errorMessage(error) })
    } finally {
      job.current = null
    }
  }

  const exportToFile = () =>
    run(async () => {
      const path = await chooseFile(name, format)
      if (!path) return null
      if (source.kind === 'rows') {
        const rows = await exportApi.exportRows(path, format, sent(), source)
        return { state: 'saved', rows, path }
      }
      const jobId = crypto.randomUUID()
      job.current = { id: jobId, stopped: false }
      const stopListening = await exportApi.onProgress(jobId, (rows) =>
        setStatus((current) => (current.state === 'working' ? { ...current, rows } : current)),
      )
      try {
        const rows = await exportApi.exportTable({
          jobId,
          connectionId: source.connectionId,
          table: source.table,
          filter: filtered && scope === 'filtered' ? source.filter : null,
          format,
          options: sent(),
          path,
        })
        return { state: 'saved', rows, path }
      } finally {
        stopListening()
      }
    })

  /** Stops the running export; the run then ends as stopped. */
  const stop = () => {
    const running = job.current
    if (!running || running.stopped) return
    running.stopped = true
    void exportApi.cancel(running.id)
  }

  const close = () => {
    stop()
    onClose()
  }

  const copy = () =>
    source.kind === 'rows' &&
    run(async () => {
      // WebKit only lets a click write to the clipboard while it is handled, so the
      // write starts now with the text still on its way from the backend.
      const text = exportApi
        .formatRows(format, sent(), source)
        .then((t) => new Blob([t], { type: 'text/plain' }))
      await navigator.clipboard.write([new ClipboardItem({ 'text/plain': text })])
      return { state: 'copied' }
    })

  const reveal = (path: string) =>
    void exportApi.reveal(path).catch((error) => setStatus({ state: 'failed', error: errorMessage(error) }))

  return (
    <Modal title={m.title} onClose={close} className={styles.dialog}>
      <div className={styles.body}>
        <p className={styles.source}>
          <Icon name={source.kind === 'table' ? 'table' : 'code'} />
          {source.kind === 'table' ? (
            <>
              <strong>
                {source.table.schema}.{source.table.name}
              </strong>
              {!filtered && <span className="muted">· {m.everyRow}</span>}
            </>
          ) : (
            <>
              <strong>{m.queryResult}</strong>
              <span className="muted">· {m.rowsShown(source.rows.length.toLocaleString('en-US'))}</span>
            </>
          )}
        </p>

        {filtered && (
          <section className={styles.section}>
            <h3 className={styles.sectionTitle} id="export-scope">
              {m.scope}
            </h3>
            <OptionList role="radiogroup" aria-labelledby="export-scope">
              <Option
                type="radio"
                name="export-scope"
                icon="search"
                title={m.scopeFiltered}
                description={m.scopeFilteredDescription}
                checked={scope === 'filtered'}
                disabled={working}
                onChange={() => setScope('filtered')}
              />
              <Option
                type="radio"
                name="export-scope"
                icon="table"
                title={m.scopeAll}
                description={m.scopeAllDescription}
                checked={scope === 'all'}
                disabled={working}
                onChange={() => setScope('all')}
              />
            </OptionList>
          </section>
        )}

        <section className={styles.section}>
          <h3 className={styles.sectionTitle} id="export-format">
            {m.format}
          </h3>
          <div role="radiogroup" aria-labelledby="export-format" className={styles.formats}>
            {FORMATS.map((option) => (
              <button
                key={option.id}
                type="button"
                role="radio"
                aria-checked={format === option.id}
                data-autofocus={format === option.id || undefined}
                className={styles.format}
                disabled={working}
                onClick={() => choose({ format: option.id })}
              >
                <span className={styles.formatIcon}>
                  <Icon name={FORMAT_ICONS[option.id]} size={15} />
                </span>
                <span className={styles.formatText}>
                  <span className={styles.formatName}>
                    {option.label}
                    <span className={styles.extension}>.{option.extension}</span>
                  </span>
                  <span className={styles.description}>
                    {option.id === 'json' && options.jsonStyle === 'lines'
                      ? m.jsonLinesDescription
                      : m[`${option.id}Description`]}
                  </span>
                </span>
              </button>
            ))}
          </div>
        </section>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>{m.options}</h3>
          <fieldset className={styles.fieldset} disabled={working}>
            <FormatOptionsFields
              format={format}
              options={options}
              onChange={(next) => choose({ options: next })}
            />
          </fieldset>
        </section>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>{m.columns}</h3>
          <ColumnPicker columns={columns} chosen={chosenColumns} onChange={setChosen} disabled={working} />
        </section>

        <ExportStatus status={status} streams={source.kind === 'table'} />
      </div>

      <footer className={styles.footer}>
        {status.state === 'saved' ? (
          <>
            <Button onClick={() => reveal(status.path)}>
              <Icon name="folder" />
              {m.reveal(isMac())}
            </Button>
            <Button variant="primary" onClick={onClose}>
              <Icon name="check" />
              {m.done}
            </Button>
          </>
        ) : (
          <>
            {working && source.kind === 'table' ? (
              <Button onClick={stop}>
                <Icon name="stop" />
                {m.stop}
              </Button>
            ) : (
              <Button onClick={close}>{m.cancel}</Button>
            )}
            {source.kind === 'rows' && (
              <Button disabled={!ready} onClick={copy}>
                <Icon name="duplicate" />
                {m.copyToClipboard}
              </Button>
            )}
            <Button variant="primary" disabled={!ready} onClick={exportToFile}>
              <Icon name="download" />
              {working ? m.exporting : m.export}
            </Button>
          </>
        )}
      </footer>
    </Modal>
  )
}

/**
 * The box under the form: the export in progress (rows written and the time
 * so far), then how it ended. Its space is kept while empty, so the dialog
 * doesn't jump when an export starts or ends.
 */
function ExportStatus({ status, streams }: { status: Status; streams: boolean }) {
  const m = useMessages(messages)
  switch (status.state) {
    case 'idle':
      return <CalloutSpace />
    case 'working':
      return <Running rows={status.rows} startedAt={status.startedAt} streams={streams} />
    case 'saved':
      return (
        <Callout
          tone="done"
          title={m.exportedRows(status.rows.toLocaleString('en-US'))}
          detail={status.path}
          mono
        />
      )
    case 'copied':
      return <Callout tone="done" title={m.copied} />
    case 'stopped':
      return <Callout tone="stopped" title={m.stopped} detail={m.stoppedDetail} />
    case 'failed':
      return <Callout tone="failed" title={m.failed} detail={status.error} mono />
  }
}

function Running({ rows, startedAt, streams }: { rows: number; startedAt: number; streams: boolean }) {
  const m = useMessages(messages)
  const elapsed = useElapsed(startedAt)
  return (
    <Callout
      tone="progress"
      title={m.exporting}
      aside={elapsed}
      // Rows already in the UI are written at once; a table's are counted as they go.
      detail={streams ? m.rowsWritten(rows.toLocaleString('en-US')) : ' '}
    />
  )
}

/**
 * The source's column names, in order: a query result has them; a table's
 * are read from its structure. `null` while loading, `undefined` when they
 * can't be read (then every column is exported).
 */
function useColumns(source: ExportSource): string[] | null | undefined {
  const [loaded, setLoaded] = useState<string[] | null | undefined>(null)
  const connectionId = source.kind === 'table' ? source.connectionId : null
  const table = source.kind === 'table' ? source.table : null
  useEffect(() => {
    if (!connectionId || !table) return
    let current = true
    structureApi.get(connectionId, table).then(
      (structure) => current && setLoaded(structure.columns.map((column) => column.name)),
      () => current && setLoaded(undefined),
    )
    return () => {
      current = false
    }
  }, [connectionId, table])
  return source.kind === 'rows' ? source.columns.map((column) => column.name) : loaded
}
