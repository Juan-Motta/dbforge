import { Button } from '../../components/Button'
import { Callout } from '../../components/Callout'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { Update } from './api'
import { messages } from './messages'
import { readNotes } from './notes'
import type { UpdateState } from './useUpdates'
import styles from './UpdateDialog.module.css'

interface UpdateDialogProps {
  state: Exclude<UpdateState, { state: 'idle' }>
  onInstall: (update: Update) => void
  onRetry: () => void
  onClose: () => void
}

/** Megabytes, with one decimal. */
function megabytes(bytes: number) {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/** What the update check found, and the update's download and install. */
export function UpdateDialog({ state, onInstall, onRetry, onClose }: UpdateDialogProps) {
  const m = useMessages(messages)
  const update = 'update' in state ? state.update : null
  const installing = state.state === 'installing'

  return (
    <Modal title={m.title} onClose={installing ? () => {} : onClose} className={styles.dialog}>
      <div className={styles.body}>
        <div className={styles.product}>
          <span className={styles.mark} aria-hidden="true">
            <Icon name="database" size={22} />
          </span>
          <div>
            {state.state === 'checking' && <p className={styles.headline}>{m.checking}</p>}
            {state.state === 'upToDate' && <p className={styles.headline}>{m.upToDate}</p>}
            {update && <p className={styles.headline}>{m.available(update.version)}</p>}
            {update && <p className={styles.detail}>{m.current(update.currentVersion)}</p>}
            {state.state === 'failed' && !update && <p className={styles.headline}>{m.failed}</p>}
          </div>
        </div>

        {update && (
          <section className={styles.section}>
            <h3 className={styles.sectionTitle}>{m.notes}</h3>
            <div className={styles.notes}>
              <Notes markdown={update.body ?? ''} empty={m.noNotes} />
            </div>
          </section>
        )}

        {state.state === 'installing' && (
          <Callout
            tone="progress"
            title={m.downloading}
            detail={
              state.total
                ? m.downloaded(megabytes(state.downloaded), megabytes(state.total))
                : m.downloadedSoFar(megabytes(state.downloaded))
            }
          />
        )}
        {state.state === 'failed' && <Callout tone="failed" title={m.failed} detail={state.error} mono />}
      </div>

      <footer className={styles.footer}>
        {state.state === 'available' && (
          <>
            <Button onClick={onClose}>{m.later}</Button>
            <Button variant="primary" data-autofocus onClick={() => onInstall(state.update)}>
              <Icon name="download" />
              {m.install}
            </Button>
          </>
        )}
        {state.state === 'installing' && (
          <Button variant="primary" disabled>
            <Icon name="download" />
            {m.install}
          </Button>
        )}
        {state.state === 'failed' && (
          <>
            <Button onClick={onClose}>{m.later}</Button>
            <Button variant="primary" onClick={() => (state.update ? onInstall(state.update) : onRetry())}>
              {m.retry}
            </Button>
          </>
        )}
        {(state.state === 'upToDate' || state.state === 'checking') && (
          <Button variant="primary" data-autofocus disabled={state.state === 'checking'} onClick={onClose}>
            {m.ok}
          </Button>
        )}
      </footer>
    </Modal>
  )
}

function Notes({ markdown, empty }: { markdown: string; empty: string }) {
  const blocks = readNotes(markdown)
  if (blocks.length === 0) return <p className={styles.empty}>{empty}</p>
  return (
    <>
      {blocks.map((block, i) =>
        block.kind === 'heading' ? (
          <h4 key={i} className={styles.noteHeading}>
            {block.text}
          </h4>
        ) : (
          <p key={i} className={block.kind === 'item' ? styles.noteItem : styles.noteText}>
            {block.runs.map((run, j) =>
              run.bold ? <strong key={j}>{run.text}</strong> : <span key={j}>{run.text}</span>,
            )}
          </p>
        ),
      )}
    </>
  )
}
