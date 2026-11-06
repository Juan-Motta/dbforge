import { useState } from 'react'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import type { TableRef } from '../../domain/table'
import { errorMessage } from '../../lib/errors'
import { chooseFile, exportApi, type RowsSource } from './api'
import { FORMATS, type ExportFormat } from './formats'
import styles from './ExportDialog.module.css'

/** A whole table, streamed from the server, or rows the UI already holds. */
export type ExportSource =
  { kind: 'table'; connectionId: string; table: TableRef } | ({ kind: 'rows' } & RowsSource)

type Status =
  | { state: 'idle' }
  | { state: 'working' }
  | { state: 'saved'; rows: number; path: string }
  | { state: 'copied' }
  | { state: 'failed'; error: string }

interface ExportDialogProps {
  source: ExportSource
  onClose: () => void
}

export function ExportDialog({ source, onClose }: ExportDialogProps) {
  const [format, setFormat] = useState<ExportFormat>('csv')
  const [status, setStatus] = useState<Status>({ state: 'idle' })
  const name = source.kind === 'table' ? source.table.name : source.name
  const working = status.state === 'working'

  const run = async (task: () => Promise<Status | null>) => {
    setStatus({ state: 'working' })
    try {
      setStatus((await task()) ?? { state: 'idle' })
    } catch (error) {
      setStatus({ state: 'failed', error: errorMessage(error) })
    }
  }

  const exportToFile = () =>
    run(async () => {
      const path = await chooseFile(name, format)
      if (!path) return null
      const rows =
        source.kind === 'table'
          ? await exportApi.exportTable(source.connectionId, source.table, format, path)
          : await exportApi.exportRows(path, format, source)
      return { state: 'saved', rows, path }
    })

  const copy = () =>
    source.kind === 'rows' &&
    run(async () => {
      // WebKit only lets a click write to the clipboard while it is handled, so the
      // write starts now with the text still on its way from the backend.
      const text = exportApi.formatRows(format, source).then((t) => new Blob([t], { type: 'text/plain' }))
      await navigator.clipboard.write([new ClipboardItem({ 'text/plain': text })])
      return { state: 'copied' }
    })

  return (
    <Modal title="Export data" onClose={onClose} className={styles.dialog}>
      <div className={styles.body}>
        <div>
          <span className={styles.label}>Source</span>
          <p className={styles.source}>
            <Icon name={source.kind === 'table' ? 'table' : 'code'} />
            {source.kind === 'table' ? (
              <>
                {source.table.schema}.{source.table.name}
                <span className="muted">· every row</span>
              </>
            ) : (
              <>
                Query result
                <span className="muted">· {source.rows.length.toLocaleString('en-US')} rows shown</span>
              </>
            )}
          </p>
        </div>

        <div>
          <span className={styles.label} id="export-format">
            Format
          </span>
          <div role="radiogroup" aria-labelledby="export-format" className={styles.formats}>
            {FORMATS.map((option) => (
              <button
                key={option.id}
                type="button"
                role="radio"
                aria-checked={format === option.id}
                data-autofocus={format === option.id || undefined}
                className={styles.format}
                onClick={() => {
                  setFormat(option.id)
                  setStatus({ state: 'idle' })
                }}
              >
                <span className={styles.extension}>.{option.extension}</span>
                <strong>{option.label}</strong>
                <span className={styles.description}>{option.description}</span>
              </button>
            ))}
          </div>
        </div>

        <p className={styles.status} role="status">
          {status.state === 'working' && (source.kind === 'table' ? 'Exporting every row…' : 'Working…')}
          {status.state === 'saved' && (
            <>
              Exported {status.rows.toLocaleString('en-US')} rows to <code>{status.path}</code>
            </>
          )}
          {status.state === 'copied' && 'Copied to the clipboard.'}
          {status.state === 'failed' && <span className={styles.error}>{status.error}</span>}
        </p>
      </div>

      <footer className={styles.footer}>
        <Button onClick={onClose}>{status.state === 'saved' ? 'Done' : 'Cancel'}</Button>
        {source.kind === 'rows' && (
          <Button disabled={working} onClick={copy}>
            Copy to clipboard
          </Button>
        )}
        <Button variant="primary" disabled={working} onClick={exportToFile}>
          <Icon name="download" />
          Export…
        </Button>
      </footer>
    </Modal>
  )
}
