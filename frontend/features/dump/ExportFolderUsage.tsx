import { useEffect, useState } from 'react'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { formatBytes } from '../table-structure/format'
import { dumpApi, type ExportFiles } from './api'
import { messages } from './messages'
import styles from './DumpDialogs.module.css'

type State =
  | { step: 'measuring' }
  | { step: 'shown'; files: ExportFiles; note?: string }
  | { step: 'confirming'; files: ExportFiles }
  | { step: 'failed'; error: string }

/**
 * How much the exports in the folder take, and deleting them: the first press
 * asks, with the count and size, the second deletes. One row, so the settings
 * don't move between the steps. Keyed by the folder, so a new one is measured afresh.
 */
export function ExportFolderUsage({ folder }: { folder: string }) {
  const m = useMessages(messages)
  const [state, setState] = useState<State>({ step: 'measuring' })

  useEffect(() => {
    let current = true
    dumpApi
      .folderUsage(folder)
      .then((files) => current && setState({ step: 'shown', files }))
      .catch((error) => current && setState({ step: 'failed', error: errorMessage(error) }))
    return () => {
      current = false
    }
  }, [folder])

  const clear = async () => {
    try {
      const removed = await dumpApi.clearFolder(folder)
      const files = await dumpApi.folderUsage(folder)
      setState({ step: 'shown', files, note: m.cleared(removed.count, formatBytes(removed.bytes)) })
    } catch (error) {
      setState({ step: 'failed', error: errorMessage(error) })
    }
  }

  const text =
    state.step === 'measuring'
      ? m.measuring
      : state.step === 'failed'
        ? state.error
        : state.step === 'confirming'
          ? m.confirmClear(state.files.count, formatBytes(state.files.bytes))
          : (state.note ?? m.exportFiles(state.files.count, formatBytes(state.files.bytes)))

  return (
    <div className={`${styles.usage} ${state.step === 'confirming' ? styles.usageConfirm : ''}`}>
      <Icon name={state.step === 'confirming' ? 'warning' : 'database'} size={14} />
      <span className={styles.usageText} role="status">
        {text}
      </span>
      {state.step === 'confirming' ? (
        <>
          <Button variant="ghost" onClick={() => setState({ step: 'shown', files: state.files })}>
            {m.cancel}
          </Button>
          <Button variant="primary" onClick={() => void clear()}>
            <Icon name="trash" />
            {m.deleteExports}
          </Button>
        </>
      ) : (
        <Button
          disabled={state.step !== 'shown' || state.files.count === 0}
          onClick={() => state.step === 'shown' && setState({ step: 'confirming', files: state.files })}
        >
          {m.clearFolder}
        </Button>
      )}
    </div>
  )
}
