import { useState, type KeyboardEvent } from 'react'
import Markdown from 'react-markdown'
import { Button, IconButton } from '../../components/Button'
import { ResizeHandle } from '../../components/ResizeHandle'
import { usePanelWidth } from '../../components/usePanelWidth'
import { CodeEditor } from '../../components/code/CodeEditor'
import { Icon } from '../../components/Icon'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { Column } from './api'
import { messages } from './messages'
import { compactJson, isJsonDocument, jsonError, prettyJson } from './cellText'
import styles from './CellInspector.module.css'

type Mode = 'text' | 'json' | 'markdown'

export interface InspectedCell {
  column: Column
  value: string | null
  /** A new row's column left to its DEFAULT. */
  unset: boolean
  /** Why the value can't be changed here, or `null` when it can. */
  locked: string | null
}

interface CellInspectorProps {
  /** The picked cell, or `null` when none is. */
  cell: InspectedCell | null
  /** Puts the text in the cell as a pending edit. */
  onSave: (value: string) => void
  onClose: () => void
}

/** A side panel with a cell's whole value: as text, as indented JSON, or as rendered Markdown. */
export function CellInspector({ cell, onSave, onClose }: CellInspectorProps) {
  const m = useMessages(messages)
  const [wrap, setWrap] = useState(true)
  const [width, setWidth, resetWidth] = usePanelWidth('dbforge.inspectorWidth', 380, 260, 900)
  return (
    <aside className={styles.panel} style={{ width }} aria-label={m.cellValue}>
      <ResizeHandle edge="left" label={m.resizePanel} onResize={setWidth} onReset={resetWidth} />
      {cell ? (
        <Value cell={cell} wrap={wrap} onWrap={() => setWrap(!wrap)} onSave={onSave} onClose={onClose} />
      ) : (
        <>
          <header className={styles.bar}>
            <span className={styles.title}>{m.value}</span>
            <span className={styles.spacer} />
            <IconButton label={m.close} onClick={onClose}>
              <Icon name="close" />
            </IconButton>
          </header>
          <p className={styles.empty}>{m.pickCell}</p>
        </>
      )}
    </aside>
  )
}

interface ValueProps {
  cell: InspectedCell
  wrap: boolean
  onWrap: () => void
  onSave: (value: string) => void
  onClose: () => void
}

/** One cell's value; the panel mounts it again for another cell, so the draft starts over. */
function Value({ cell, wrap, onWrap, onSave, onClose }: ValueProps) {
  const m = useMessages(messages)
  const original = cell.value ?? ''
  const startsAsJson = isJsonDocument(original)
  const [mode, setMode] = useState<Mode>(startsAsJson ? 'json' : 'text')
  const [draft, setDraft] = useState(startsAsJson ? prettyJson(original) : original)

  const bothJson = isJsonDocument(draft) && startsAsJson
  // Only the layout of JSON changing is no change.
  const changed = bothJson ? compactJson(draft) !== compactJson(original) : draft !== original
  const error = mode === 'json' ? jsonError(draft) : null
  const canSave = changed && !cell.locked && error === null

  const save = () => {
    if (!canSave) return
    // A one-line JSON value stays on one line; one the database keeps laid out keeps its layout.
    onSave(bothJson && !original.includes('\n') ? compactJson(draft) : draft)
  }

  const show = (next: Mode) => {
    if (next === 'json' && isJsonDocument(draft)) setDraft(prettyJson(draft))
    setMode(next)
  }

  const onKeyDown = (event: KeyboardEvent) => {
    // ⌘S puts this value in the cell first; a second ⌘S applies every change.
    if ((event.metaKey || event.ctrlKey) && event.key === 's' && canSave) {
      event.preventDefault()
      event.stopPropagation()
      save()
    }
  }

  const status = error
    ? error
    : cell.unset
      ? 'DEFAULT'
      : cell.value === null && !changed
        ? 'NULL'
        : m.characters(draft.length.toLocaleString('en-US'))

  return (
    <div className={styles.value} onKeyDown={onKeyDown}>
      <header className={styles.bar}>
        <span className={styles.title} title={cell.column.dataType}>
          {cell.column.name}
          <span className={styles.type}>{cell.column.dataType}</span>
        </span>
        <span className={styles.spacer} />
        <div className={styles.seg} role="group" aria-label={m.showValueAs}>
          <button type="button" aria-pressed={mode === 'text'} onClick={() => show('text')}>
            {m.text}
          </button>
          <button
            type="button"
            aria-pressed={mode === 'json'}
            disabled={mode !== 'json' && !isJsonDocument(draft)}
            title={m.jsonHint}
            onClick={() => show('json')}
          >
            JSON
          </button>
          <button type="button" aria-pressed={mode === 'markdown'} onClick={() => show('markdown')}>
            Markdown
          </button>
        </div>
        <IconButton label={m.wrapLines} active={wrap} disabled={mode === 'markdown'} onClick={onWrap}>
          <Icon name="wrap" />
        </IconButton>
        <IconButton label={m.close} onClick={onClose}>
          <Icon name="close" />
        </IconButton>
      </header>

      <div className={styles.body}>
        {mode === 'markdown' ? (
          <div className={styles.markdown}>
            <Markdown
              skipHtml
              components={{
                // Links would take the app's window away; images would load from anywhere.
                a: ({ children, href }) => (
                  <span className={styles.link} title={href}>
                    {children}
                  </span>
                ),
                img: ({ alt }) => <span className={styles.image}>[{alt || m.image}]</span>,
              }}
            >
              {draft}
            </Markdown>
          </div>
        ) : (
          <CodeEditor
            value={draft}
            onChange={setDraft}
            language={mode}
            readOnly={cell.locked !== null}
            wrap={wrap}
            label={m.valueOf(cell.column.name)}
          />
        )}
      </div>

      <footer className={styles.footer}>
        <span className={error ? styles.error : styles.status} title={status}>
          {status}
        </span>
        <span className={styles.spacer} />
        <Button disabled={!changed} onClick={() => setDraft(startsAsJson ? prettyJson(original) : original)}>
          {m.revert}
        </Button>
        <Button variant="primary" disabled={!canSave} title={cell.locked ?? undefined} onClick={save}>
          {m.save}
        </Button>
      </footer>
    </div>
  )
}
