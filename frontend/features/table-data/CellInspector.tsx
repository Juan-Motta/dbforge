import { useState, type KeyboardEvent } from 'react'
import Markdown from 'react-markdown'
import { Button, IconButton } from '../../components/Button'
import { ResizeHandle } from '../../components/ResizeHandle'
import { usePanelWidth } from '../../components/usePanelWidth'
import { CodeEditor } from '../../components/code/CodeEditor'
import { Icon } from '../../components/Icon'
import type { Column } from './api'
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
  const [wrap, setWrap] = useState(true)
  const [width, setWidth, resetWidth] = usePanelWidth('dbforge.inspectorWidth', 380, 260, 900)
  return (
    <aside className={styles.panel} style={{ width }} aria-label="Cell value">
      <ResizeHandle edge="left" label="Resize the panel" onResize={setWidth} onReset={resetWidth} />
      {cell ? (
        <Value cell={cell} wrap={wrap} onWrap={() => setWrap(!wrap)} onSave={onSave} onClose={onClose} />
      ) : (
        <>
          <header className={styles.bar}>
            <span className={styles.title}>Value</span>
            <span className={styles.spacer} />
            <IconButton label="Close" onClick={onClose}>
              <Icon name="close" />
            </IconButton>
          </header>
          <p className={styles.empty}>Pick a cell to see its whole value.</p>
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
        : `${draft.length.toLocaleString('en-US')} characters`

  return (
    <div className={styles.value} onKeyDown={onKeyDown}>
      <header className={styles.bar}>
        <span className={styles.title} title={cell.column.dataType}>
          {cell.column.name}
          <span className={styles.type}>{cell.column.dataType}</span>
        </span>
        <span className={styles.spacer} />
        <div className={styles.seg} role="group" aria-label="Show value as">
          <button type="button" aria-pressed={mode === 'text'} onClick={() => show('text')}>
            Text
          </button>
          <button
            type="button"
            aria-pressed={mode === 'json'}
            disabled={mode !== 'json' && !isJsonDocument(draft)}
            title="Indented, for a JSON object or array"
            onClick={() => show('json')}
          >
            JSON
          </button>
          <button type="button" aria-pressed={mode === 'markdown'} onClick={() => show('markdown')}>
            Markdown
          </button>
        </div>
        <IconButton label="Wrap long lines" active={wrap} disabled={mode === 'markdown'} onClick={onWrap}>
          <Icon name="wrap" />
        </IconButton>
        <IconButton label="Close" onClick={onClose}>
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
                img: ({ alt }) => <span className={styles.image}>[{alt || 'image'}]</span>,
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
            label={`Value of ${cell.column.name}`}
          />
        )}
      </div>

      <footer className={styles.footer}>
        <span className={error ? styles.error : styles.status} title={status}>
          {status}
        </span>
        <span className={styles.spacer} />
        <Button disabled={!changed} onClick={() => setDraft(startsAsJson ? prettyJson(original) : original)}>
          Revert
        </Button>
        <Button variant="primary" disabled={!canSave} title={cell.locked ?? undefined} onClick={save}>
          Save
        </Button>
      </footer>
    </div>
  )
}
