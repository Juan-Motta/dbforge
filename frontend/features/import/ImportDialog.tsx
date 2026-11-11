import { useEffect, useRef, useState } from 'react'
import { Button } from '../../components/Button'
import { Callout, CalloutSpace } from '../../components/Callout'
import { DataGrid } from '../../components/data-grid/DataGrid'
import { Icon, type IconName } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { Select } from '../../components/Select'
import { TextField } from '../../components/TextField'
import { useElapsed } from '../../components/useElapsed'
import type { TableRef } from '../../domain/table'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { tableDataApi } from '../table-data/api'
import { NO_FILTER } from '../table-data/rowFilter'
import {
  chooseDataFile,
  DEFAULT_READ_OPTIONS,
  importApi,
  type FilePreview,
  type ImportFormat,
  type ReadOptions,
} from './api'
import { formatOfPath, mapByName, repeatedColumns } from './mapping'
import { messages } from './messages'
import styles from './ImportDialog.module.css'

interface ImportDialogProps {
  connectionId: string
  table: TableRef
  /** SQL engines import in one transaction; document databases a batch at a time. */
  transactional: boolean
  /** The tag (e.g. PROD) that makes the import ask first; `null` when it doesn't. */
  confirmTag: string | null
  onClose: () => void
  /** Rows went in: whatever shows the table's rows reads them again. */
  onImported: () => void
}

type Status =
  | { state: 'idle' }
  | { state: 'confirming' }
  | { state: 'working'; rows: number; startedAt: number }
  | { state: 'done'; rows: number }
  | { state: 'stopped'; rows: number }
  | { state: 'failed'; error: string }

/** What the file holds: being read, read, or not readable with the options chosen. */
type Read =
  | { state: 'none' }
  | { state: 'reading' }
  | { state: 'read'; preview: FilePreview }
  | { state: 'failed'; error: string }

const FORMATS: { id: ImportFormat; label: string; icon: IconName }[] = [
  { id: 'csv', label: 'CSV', icon: 'table' },
  { id: 'tsv', label: 'TSV', icon: 'column' },
  { id: 'json', label: 'JSON', icon: 'code' },
]

/** Rows the preview grid shows. */
const PREVIEW_ROWS = 5

/** Adds the rows of a CSV, TSV or JSON file to a table, with each field mapped to a column. */
export function ImportDialog({
  connectionId,
  table,
  transactional,
  confirmTag,
  onClose,
  onImported,
}: ImportDialogProps) {
  const m = useMessages(messages)
  const [path, setPath] = useState<string | null>(null)
  const [format, setFormat] = useState<ImportFormat>('csv')
  const [options, setOptions] = useState<ReadOptions>(DEFAULT_READ_OPTIONS)
  const read = useFilePreview(path, format, options)
  const fields = read.state === 'read' ? read.preview.fields : []
  const tableColumns = useTableColumns(connectionId, table)
  // A document takes fields the collection doesn't have yet, so those are offered too.
  const columns =
    tableColumns &&
    (transactional ? tableColumns : [...tableColumns, ...fields.filter((f) => !tableColumns.includes(f))])
  // Chosen by the user for these fields; until then, by name.
  const [chosen, setChosen] = useState<{ fields: string[]; mapping: (string | null)[] } | null>(null)
  const mapping =
    chosen && sameFields(chosen.fields, fields) ? chosen.mapping : columns ? mapByName(fields, columns) : []
  const repeated = repeatedColumns(mapping)
  const [status, setStatus] = useState<Status>({ state: 'idle' })
  // The running import: whether the user stopped it, and the last count it reported.
  const job = useRef<{ id: string; stopped: boolean; rows: number } | null>(null)
  const working = status.state === 'working'
  const mapped = mapping.some((column) => column !== null)
  const ready = !working && read.state === 'read' && columns !== null && mapped && repeated.length === 0
  const relation = `${table.schema}.${table.name}`

  const pickFile = async () => {
    const picked = await chooseDataFile(m.dataFiles)
    if (!picked) return
    setPath(picked)
    setFormat(formatOfPath(picked))
    setStatus({ state: 'idle' })
  }

  const start = async () => {
    if (!ready || read.state !== 'read') return
    if (confirmTag && status.state !== 'confirming') {
      setStatus({ state: 'confirming' })
      return
    }
    const jobId = crypto.randomUUID()
    const running = { id: jobId, stopped: false, rows: 0 }
    job.current = running
    setStatus({ state: 'working', rows: 0, startedAt: Date.now() })
    const stopListening = await importApi.onProgress(jobId, (rows) => {
      running.rows = rows
      setStatus((current) => (current.state === 'working' ? { ...current, rows } : current))
    })
    try {
      const rows = await importApi.importTable({
        jobId,
        connectionId,
        table,
        path: path!,
        format,
        options,
        fields: read.preview.fields,
        columns: mapping,
      })
      setStatus({ state: 'done', rows })
      onImported()
    } catch (error) {
      if (running.stopped) {
        setStatus({ state: 'stopped', rows: running.rows })
        if (!transactional && running.rows > 0) onImported()
      } else {
        setStatus({ state: 'failed', error: errorMessage(error) })
        // A document database keeps the rows in before the error; the error says how many.
        if (!transactional) onImported()
      }
    } finally {
      stopListening()
      job.current = null
    }
  }

  const stop = () => {
    const running = job.current
    if (!running || running.stopped) return
    running.stopped = true
    void importApi.cancel(running.id)
  }

  const close = () => {
    stop()
    onClose()
  }

  const setColumn = (field: number, column: string | null) =>
    setChosen({ fields, mapping: mapping.map((current, i) => (i === field ? column : current)) })

  return (
    <Modal title={m.title} onClose={close} className={styles.dialog}>
      <div className={styles.body}>
        <p className={styles.source}>
          <Icon name="table" />
          <strong>{relation}</strong>
        </p>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>{m.file}</h3>
          <div className={styles.fileRow}>
            <Icon name="folder" size={15} />
            <span className={styles.path} title={path ?? undefined}>
              <bdi>{path ?? m.noFile}</bdi>
            </span>
            <Button disabled={working} onClick={() => void pickFile()}>
              {m.choose}
            </Button>
          </div>
        </section>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle} id="import-format">
            {m.format}
          </h3>
          <div role="radiogroup" aria-labelledby="import-format" className={styles.formats}>
            {FORMATS.map((option) => (
              <button
                key={option.id}
                type="button"
                role="radio"
                aria-checked={format === option.id}
                className={styles.format}
                disabled={working}
                onClick={() => setFormat(option.id)}
              >
                <span className={styles.formatIcon}>
                  <Icon name={option.icon} size={15} />
                </span>
                <span className={styles.formatText}>
                  <span className={styles.formatName}>{option.label}</span>
                  <span className={styles.description}>{m[`${option.id}Description`]}</span>
                </span>
              </button>
            ))}
          </div>
        </section>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>{m.options}</h3>
          <fieldset className={styles.fieldset} disabled={working}>
            {format === 'json' ? (
              <div className={styles.options}>
                <p className={styles.note}>{m.jsonNote}</p>
              </div>
            ) : (
              <div className={format === 'csv' ? styles.optionsThree : styles.options}>
                <Select
                  label={m.header}
                  value={options.header ? 'names' : 'data'}
                  options={[
                    { value: 'names', label: m.headerNames },
                    { value: 'data', label: m.headerData },
                  ]}
                  onChange={(value) => setOptions({ ...options, header: value === 'names' })}
                />
                <TextField
                  label={m.nullText}
                  mono
                  value={options.nullText}
                  placeholder={m.nullTextPlaceholder}
                  onChange={(event) => setOptions({ ...options, nullText: event.target.value })}
                />
                {format === 'csv' && (
                  <Select
                    label={m.delimiter}
                    value={options.delimiter}
                    options={[
                      { value: ',', label: m.comma },
                      { value: ';', label: m.semicolon },
                      { value: '|', label: m.pipe },
                    ]}
                    onChange={(delimiter) => setOptions({ ...options, delimiter })}
                  />
                )}
              </div>
            )}
          </fieldset>
        </section>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>{m.preview}</h3>
          <div className={styles.preview}>
            {read.state === 'read' && read.preview.rows.length > 0 ? (
              <DataGrid
                columns={read.preview.fields.map((name) => ({ name, dataType: '' }))}
                rows={read.preview.rows.slice(0, PREVIEW_ROWS)}
              />
            ) : (
              <p className={read.state === 'failed' ? styles.error : styles.empty}>
                {read.state === 'none'
                  ? m.previewEmpty
                  : read.state === 'reading'
                    ? m.readingFile
                    : read.state === 'failed'
                      ? read.error
                      : m.noRows}
              </p>
            )}
          </div>
        </section>

        {read.state === 'read' && (
          <section className={styles.section}>
            <h3 className={styles.sectionTitle}>{m.mapping}</h3>
            {columns === null ? (
              <p className={styles.empty}>{m.loadingColumns}</p>
            ) : (
              <div className={styles.mapping}>
                {fields.map((field, i) => (
                  <div key={`${i}:${field}`} className={styles.mappingRow}>
                    <span className={styles.field} title={field}>
                      {field}
                    </span>
                    <Icon name="chevronRight" size={13} />
                    <Select
                      label={`${field} ${m.column}`}
                      hideLabel
                      value={mapping[i] ?? ''}
                      options={[
                        { value: '', label: m.leaveOut },
                        ...columns.map((column) => ({ value: column, label: column })),
                      ]}
                      onChange={(value) => setColumn(i, value || null)}
                    />
                  </div>
                ))}
              </div>
            )}
            {repeated.length > 0 && <p className={styles.error}>{m.repeated(repeated.join(', '))}</p>}
            {columns !== null && !mapped && <p className={styles.error}>{m.nothingMapped}</p>}
          </section>
        )}

        <p className={styles.note}>{transactional ? m.transaction : m.batches}</p>

        <ImportStatus
          status={status}
          transactional={transactional}
          confirmTag={confirmTag}
          relation={relation}
        />
      </div>

      <footer className={styles.footer}>
        {status.state === 'done' ? (
          <Button variant="primary" onClick={onClose}>
            <Icon name="check" />
            {m.done}
          </Button>
        ) : (
          <>
            {working ? (
              <Button onClick={stop}>
                <Icon name="stop" />
                {m.stop}
              </Button>
            ) : (
              <Button onClick={close}>{m.cancel}</Button>
            )}
            <Button variant="primary" disabled={!ready} onClick={() => void start()}>
              <Icon name="upload" />
              {working
                ? m.importing
                : status.state === 'confirming' && confirmTag
                  ? m.importInto(confirmTag)
                  : m.import}
            </Button>
          </>
        )}
      </footer>
    </Modal>
  )
}

/**
 * The box under the form: a protected connection's warning, the import in
 * progress, then how it ended. Its space is kept while empty.
 */
function ImportStatus({
  status,
  transactional,
  confirmTag,
  relation,
}: {
  status: Status
  transactional: boolean
  confirmTag: string | null
  relation: string
}) {
  const m = useMessages(messages)
  const count = (rows: number) => rows.toLocaleString('en-US')
  switch (status.state) {
    case 'idle':
      return <CalloutSpace />
    case 'confirming':
      return <Callout tone="warning" title={m.confirm(confirmTag ?? '')} detail={m.confirmDetail(relation)} />
    case 'working':
      return <Running rows={status.rows} startedAt={status.startedAt} />
    case 'done':
      return <Callout tone="done" title={m.imported(count(status.rows))} />
    case 'stopped':
      return (
        <Callout
          tone="stopped"
          title={m.stopped}
          detail={transactional || status.rows === 0 ? m.stoppedNothing : m.stoppedSome(count(status.rows))}
        />
      )
    case 'failed':
      return (
        <Callout
          tone="failed"
          title={m.failed}
          detail={transactional ? `${status.error} ${m.failedNothing}` : status.error}
          mono
        />
      )
  }
}

function Running({ rows, startedAt }: { rows: number; startedAt: number }) {
  const m = useMessages(messages)
  const elapsed = useElapsed(startedAt)
  return (
    <Callout
      tone="progress"
      title={m.importing}
      aside={elapsed}
      detail={m.rowsImported(rows.toLocaleString('en-US'))}
    />
  )
}

function sameFields(a: string[], b: string[]) {
  return a.length === b.length && a.every((field, i) => field === b[i])
}

/** The file's fields and first rows, read again whenever the file, format or options change. */
function useFilePreview(path: string | null, format: ImportFormat, options: ReadOptions): Read {
  const [result, setResult] = useState<{ key: string; read: Read } | null>(null)
  const key = JSON.stringify([path, format, options])
  useEffect(() => {
    if (!path) return
    let current = true
    importApi.preview(path, format, options).then(
      (preview) => current && setResult({ key, read: { state: 'read', preview } }),
      (error) => current && setResult({ key, read: { state: 'failed', error: errorMessage(error) } }),
    )
    return () => {
      current = false
    }
    // `key` covers the options by value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])
  if (!path) return { state: 'none' }
  return result?.key === key ? result.read : { state: 'reading' }
}

/** The columns a value can be imported into: all but the ones the database computes. */
function useTableColumns(connectionId: string, table: TableRef): string[] | null {
  const [columns, setColumns] = useState<string[] | null>(null)
  useEffect(() => {
    let current = true
    // One row is read for the columns, which say which ones the database computes.
    tableDataApi.fetchRows(connectionId, table, NO_FILTER, 0, 1).then(
      (page) =>
        current &&
        setColumns(page.columns.filter((column) => !column.generated).map((column) => column.name)),
      // Without the structure no column can be offered; a document database still takes the file's fields.
      () => current && setColumns([]),
    )
    return () => {
      current = false
    }
  }, [connectionId, table])
  return columns
}
