import { useState, type KeyboardEvent, type ReactNode } from 'react'
import Markdown from 'react-markdown'
import { Button, IconButton } from '../../components/Button'
import { ResizeHandle } from '../../components/ResizeHandle'
import { usePanelWidth } from '../../components/usePanelWidth'
import { CodeEditor } from '../../components/code/CodeEditor'
import { Icon } from '../../components/Icon'
import { Select, type SelectOption } from '../../components/Select'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { Column } from './api'
import { messages } from './messages'
import { compactJson, isJsonDocument, jsonError, prettyJson } from './cellText'
import { choicesOf, type Choice } from './choices'
import { temporalKindOf, type TemporalKind } from './temporal'
import { TemporalFields } from './TemporalFields'
import { useTemporalDraft } from './useTemporalDraft'
import styles from './CellInspector.module.css'

type Mode = 'text' | 'json' | 'markdown'

export interface InspectedCell {
  column: Column
  value: string | null
  /** A new row's column left to its DEFAULT. */
  unset: boolean
  /** Why the value can't be changed here, or `null` when it can. */
  locked: string | null
  /** Whether the column takes NULL. */
  nullable: boolean
  /** The values an enum column takes. */
  options?: string[]
}

interface CellInspectorProps {
  /** The picked cell, or `null` when none is. */
  cell: InspectedCell | null
  /** Puts the value in the cell as a pending edit; `null` is SQL NULL. */
  onSave: (value: string | null) => void
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
        <TypedValue cell={cell} wrap={wrap} onWrap={() => setWrap(!wrap)} onSave={onSave} onClose={onClose} />
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
  onSave: (value: string | null) => void
  onClose: () => void
}

/** The editor the column's type calls for: a list of values, date and time controls, or text. */
function TypedValue(props: ValueProps) {
  const { dataType } = props.cell.column
  const choices = choicesOf(dataType, props.cell.options)
  if (choices) return <ChoiceValue {...props} choices={choices} />
  const kind = temporalKindOf(dataType)
  if (kind) return <TemporalValue {...props} kind={kind} />
  return <Value {...props} />
}

/** The column's name and type, and the panel's close button. */
function Header({
  cell,
  onClose,
  children,
}: {
  cell: InspectedCell
  onClose: () => void
  children?: ReactNode
}) {
  const m = useMessages(messages)
  return (
    <header className={styles.bar}>
      <span className={styles.title} title={cell.column.dataType}>
        {cell.column.name}
        <span className={styles.type}>{cell.column.dataType}</span>
      </span>
      <span className={styles.spacer} />
      {children}
      <IconButton label={m.close} onClick={onClose}>
        <Icon name="close" />
      </IconButton>
    </header>
  )
}

/** Revert and Save under a typed editor, with what the value is now. */
function Footer({
  status,
  error,
  changed,
  canSave,
  locked,
  onRevert,
  onSave,
}: {
  status: string
  error?: boolean
  changed: boolean
  canSave: boolean
  locked: string | null
  onRevert: () => void
  onSave: () => void
}) {
  const m = useMessages(messages)
  return (
    <footer className={styles.footer}>
      <span className={error ? styles.error : styles.status} title={status}>
        {status}
      </span>
      <span className={styles.spacer} />
      <Button disabled={!changed} onClick={onRevert}>
        {m.revert}
      </Button>
      <Button variant="primary" disabled={!canSave} title={locked ?? undefined} onClick={onSave}>
        {m.save}
      </Button>
    </footer>
  )
}

/** ⌘S keeps the value in the cell; a second ⌘S applies every change. */
function saveKey(save: () => void, canSave: boolean) {
  return (event: KeyboardEvent) => {
    if ((event.metaKey || event.ctrlKey) && event.key === 's' && canSave) {
      event.preventDefault()
      event.stopPropagation()
      save()
    }
  }
}

/** Stands for NULL among a list's string values. */
const NULL_CHOICE = '\u0000null'

/** A boolean or an enum: a dropdown of its values (and NULL where the column takes it). */
function ChoiceValue({ cell, onSave, onClose, choices }: ValueProps & { choices: Choice[] }) {
  const m = useMessages(messages)
  const [draft, setDraft] = useState<string | null>(cell.value)
  const changed = draft !== cell.value
  const canSave = changed && !cell.locked
  const save = () => canSave && onSave(draft)
  const options: SelectOption<string>[] = [
    ...choices.map((choice) => ({ value: choice.value, label: choice.label })),
    ...(cell.nullable || cell.value === null ? [{ value: NULL_CHOICE, label: 'NULL' }] : []),
  ]
  return (
    <div className={styles.value} onKeyDown={saveKey(save, canSave)}>
      <Header cell={cell} onClose={onClose} />
      <div className={styles.typed}>
        {cell.locked ? (
          <p className={styles.lockedValue}>{cell.value ?? 'NULL'}</p>
        ) : (
          <Select
            label={m.valueOf(cell.column.name)}
            value={draft ?? NULL_CHOICE}
            options={options}
            autoFocus
            onChange={(value) => setDraft(value === NULL_CHOICE ? null : value)}
          />
        )}
      </div>
      <Footer
        status={cell.unset && !changed ? 'DEFAULT' : (draft ?? 'NULL')}
        changed={changed}
        canSave={canSave}
        locked={cell.locked}
        onRevert={() => setDraft(cell.value)}
        onSave={save}
      />
    </div>
  )
}

/** A date or a time: its text and the same controls as the cell's editor, Now or Today, and NULL. */
function TemporalValue({ cell, onSave, onClose, kind }: ValueProps & { kind: TemporalKind }) {
  const m = useMessages(messages)
  const offsetSpace = /datetimeoffset/i.test(cell.column.dataType)
  const draft = useTemporalDraft(cell.value, kind, offsetSpace)
  const changed = draft.text !== (cell.value ?? '')
  const canSave = changed && !cell.locked && draft.value !== null
  const save = () => canSave && onSave(draft.value)
  return (
    <div className={styles.value} onKeyDown={saveKey(save, canSave)}>
      <Header cell={cell} onClose={onClose} />
      <div className={styles.typed}>
        <input
          className={styles.temporalText}
          aria-label={m.valueOf(cell.column.name)}
          aria-invalid={!draft.valid}
          readOnly={cell.locked !== null}
          spellCheck={false}
          value={draft.text}
          placeholder={cell.value === null ? 'NULL' : undefined}
          onChange={(event) => draft.type(event.target.value)}
          onKeyDown={(event) => event.key === 'Enter' && save()}
        />
        {!cell.locked && (
          <>
            <TemporalFields kind={kind} parts={draft.parts} onChange={draft.change} />
            <div className={styles.typedActions}>
              <Button onClick={draft.now}>{kind === 'date' ? m.today : m.now}</Button>
              {cell.nullable && cell.value !== null && <Button onClick={() => onSave(null)}>NULL</Button>}
            </div>
          </>
        )}
      </div>
      <Footer
        status={cell.unset && !changed ? 'DEFAULT' : draft.text || 'NULL'}
        error={!draft.valid}
        changed={changed}
        canSave={canSave}
        locked={cell.locked}
        onRevert={() => draft.type(cell.value ?? '')}
        onSave={save}
      />
    </div>
  )
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
