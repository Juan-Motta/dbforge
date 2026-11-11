import {
  memo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
} from 'react'
import { GutterNumber } from '../../components/data-grid/RowGutter'
import { Icon } from '../../components/Icon'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { Column } from './api'
import { isDefault, type CellValue, type Edits } from './edits'
import { messages } from './messages'
import { ChoiceEditor } from './ChoiceEditor'
import type { choicesOf } from './choices'
import type { temporalKindOf } from './temporal'
import { TemporalEditor } from './TemporalEditor'
import styles from './EditableGrid.module.css'

export type Value = string | null

/** A cell's row: a loaded row (by index) or a new row (by id). */
export type Editing = { row: number } | { newRow: number }

export type CellState = { edited?: boolean; unset?: boolean; locked?: boolean }

/** What every row draws its cells with: the same for all of them while the columns stay. */
export interface GridShape {
  columns: Column[]
  /** Each column's fixed width, if it has one. */
  sized: (CSSProperties | undefined)[]
  numeric: boolean[]
  editors: { temporalKind: ReturnType<typeof temporalKindOf>; choices: ReturnType<typeof choicesOf> }[]
  columnInfo?: Record<string, { nullable: boolean }>
  editable: boolean
  linkOf?: (row: number, column: number) => (() => void) | null
}

/** What a row's cells do: handlers that keep their identity, so a row is drawn again only when it changes. */
export interface GridRowActions {
  pressCell: (at: Editing, column: number, event: PointerEvent) => void
  enterCell: (at: Editing, column: number) => void
  clickCell: (at: Editing, column: number, event: MouseEvent) => void
  editCell: (at: Editing, column: number) => void
  openMenu: (at: Editing, column: number, state: CellState, event: MouseEvent) => void
  /** Holding the button on a row's number and moving over others selects whole rows. */
  pressRowNumber: (rowId: string, event: PointerEvent) => void
  enterRowNumber: (rowId: string) => void
  commit: (value: Value) => void
  cancelEdit: () => void
}

/** A row as shown: a loaded one with its pending changes, or a new one. */
export type RowSource =
  | {
      kind: 'loaded'
      index: number
      values: Value[]
      changed: Record<string, CellValue> | undefined
      deleted: boolean
    }
  | { kind: 'inserted'; insert: Edits['inserts'][number] }

/** How the selection, the editor and the menu reach a row's cells. */
interface RowMarks {
  /** Every cell of the row is selected, as its number picks it. */
  whole: boolean
  activeColumn: number | null
  /** The columns of cells selected among several, as `1,3`; empty when none. */
  selectedColumns: string
  editingColumn: number | null
  menuColumn: number | null
}

type GridRowProps = RowSource & RowMarks & { shape: GridShape; actions: GridRowActions }

/** One row of the grid; memoized, so scrolling draws only the rows that come into sight. */
export const GridRow = memo(function GridRow(props: GridRowProps) {
  const { shape, actions, whole } = props
  const selected = props.selectedColumns ? props.selectedColumns.split(',').map(Number) : []
  const marks = (column: number) => ({
    active: props.activeColumn === column,
    selected: selected.includes(column),
    editing: props.editingColumn === column,
    menu: props.menuColumn === column,
  })

  if (props.kind === 'inserted') {
    const { insert } = props
    return (
      <tr className={styles.inserted} aria-selected={whole}>
        {shape.columns.map((column, c) => (
          <GridCell
            key={c}
            shape={shape}
            actions={actions}
            at={{ newRow: insert.id }}
            column={c}
            value={insert.values[column.name] ?? null}
            state={{
              edited: column.name in insert.values,
              unset: !(column.name in insert.values),
              locked: column.generated,
            }}
            {...marks(c)}
          />
        ))}
      </tr>
    )
  }

  const { index, values, changed, deleted } = props
  return (
    <tr className={deleted ? styles.deleted : undefined} aria-selected={whole}>
      {shape.columns.map((column, c) => {
        const change = changed && column.name in changed ? changed[column.name] : undefined
        return (
          <GridCell
            key={c}
            shape={shape}
            actions={actions}
            at={{ row: index }}
            column={c}
            value={deleted || change === undefined ? values[c] : textOf(change)}
            state={
              deleted
                ? { locked: true }
                : {
                    edited: change !== undefined,
                    unset: change !== undefined && isDefault(change),
                    locked: column.generated,
                  }
            }
            {...marks(c)}
          />
        )
      })}
    </tr>
  )
})

interface RowNumberProps {
  rowId: string
  /** The number shown: the row's, or `+` for a new row and `−` for one to delete. */
  label: string | number
  state: 'loaded' | 'inserted' | 'deleted'
  /** Every cell of the row is selected. */
  whole: boolean
  /** Whether a press picks the row: new rows always, loaded ones of tables that can be edited. */
  picks: boolean
  actions: GridRowActions
}

/** A row's number in the grid's gutter; pressing it picks the whole row. */
export const RowNumber = memo(function RowNumber({
  rowId,
  label,
  state,
  whole,
  picks,
  actions,
}: RowNumberProps) {
  return (
    <GutterNumber
      className={
        [
          state === 'inserted' && styles.insertedNumber,
          state === 'deleted' && styles.deletedNumber,
          whole && styles.wholeRow,
        ]
          .filter(Boolean)
          .join(' ') || undefined
      }
      aria-selected={whole}
      onPointerDown={picks ? (event) => actions.pressRowNumber(rowId, event) : undefined}
      onPointerEnter={picks ? () => actions.enterRowNumber(rowId) : undefined}
    >
      {label}
    </GutterNumber>
  )
})

interface GridCellProps {
  shape: GridShape
  actions: GridRowActions
  at: Editing
  column: number
  value: Value
  state: CellState
  active: boolean
  selected: boolean
  editing: boolean
  menu: boolean
}

function GridCell({
  shape,
  actions,
  at,
  column,
  value,
  state,
  active,
  selected,
  editing,
  menu,
}: GridCellProps) {
  const m = useMessages(messages)
  const { columns, sized, numeric, editors } = shape
  // Edited or new values may not exist yet in the referenced table: only loaded ones link.
  const link = 'row' in at && !state.edited ? (shape.linkOf?.(at.row, column) ?? null) : null
  const classes = [
    numeric[column] && styles.numeric,
    value === null && !state.unset && styles.null,
    state.unset && styles.unset,
    state.edited && styles.edited,
  ]
  const content = link ? (
    <span className={styles.linked}>
      <span className={styles.linkedText}>{value}</span>
      <button
        type="button"
        className={styles.link}
        aria-label={m.openPointedRow(value)}
        title={m.openReferencedRow}
        onClick={(event) => {
          event.stopPropagation()
          link()
        }}
        onDoubleClick={(event) => event.stopPropagation()}
      >
        <Icon name="external" size={12} />
      </button>
    </span>
  ) : state.unset ? (
    'DEFAULT'
  ) : (
    (value ?? 'NULL')
  )

  if (editing) {
    const { temporalKind, choices } = editors[column]
    const nullable = shape.columnInfo?.[columns[column].name]?.nullable ?? true
    // The editor lies over the cell, which keeps its content (unseen) and so its size.
    return (
      <td className={[...classes, styles.editingCell].filter(Boolean).join(' ')} style={sized[column]}>
        <span className={styles.hidden} aria-hidden="true">
          {content}
        </span>
        {choices ? (
          <ChoiceEditor
            initial={value}
            choices={choices}
            nullable={nullable}
            onCommit={actions.commit}
            onCancel={actions.cancelEdit}
          />
        ) : temporalKind ? (
          <TemporalEditor
            initial={value}
            kind={temporalKind}
            offsetSpace={/datetimeoffset/i.test(columns[column].dataType)}
            nullable={nullable}
            onCommit={actions.commit}
            onCancel={actions.cancelEdit}
          />
        ) : (
          <CellEditor initial={value} onCommit={actions.commit} onCancel={actions.cancelEdit} />
        )}
      </td>
    )
  }

  return (
    <td
      className={classes.filter(Boolean).join(' ') || undefined}
      style={sized[column]}
      title={value ?? undefined}
      // Holding the button on a cell and moving over others selects the block between them.
      onPointerDown={(event) => actions.pressCell(at, column, event)}
      onPointerEnter={() => actions.enterCell(at, column)}
      // A click picks the cell; Shift and ⌘/Ctrl grow or change the selection.
      onClick={(event) => actions.clickCell(at, column, event)}
      onDoubleClick={shape.editable && !state.locked ? () => actions.editCell(at, column) : undefined}
      onContextMenu={(event) => actions.openMenu(at, column, state, event)}
      data-menu-open={menu || undefined}
      data-active={active || undefined}
      data-selected={selected || undefined}
    >
      {content}
    </td>
  )
}

/** A DEFAULT shows as an empty cell marked DEFAULT; other values as they are. */
function textOf(value: CellValue): Value {
  return isDefault(value) ? null : value
}

/** Enter or leaving the field keeps the text; Escape cancels. NULL is set from the cell's menu. */
function CellEditor({
  initial,
  onCommit,
  onCancel,
}: {
  initial: Value
  onCommit: (value: Value) => void
  onCancel: () => void
}) {
  const [text, setText] = useState(initial ?? '')
  // Closing gives the focus back to the grid, which blurs this field while it is
  // still drawn: the editor closes once, so that blur can't keep a cancelled text.
  const closed = useRef(false)
  const close = (how: () => void) => {
    if (closed.current) return
    closed.current = true
    how()
  }
  // Untouched text is no change: just opening a NULL or DEFAULT cell must not store ''.
  const finish = () => close(() => (text === (initial ?? '') ? onCancel() : onCommit(text)))
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Enter') finish()
    else if (event.key === 'Escape') close(onCancel)
    else if ((event.metaKey || event.ctrlKey) && event.key === 's') {
      // Save this cell first; applying now would leave the text being typed out.
      // A second ⌘S applies everything.
      event.preventDefault()
      event.stopPropagation()
      finish()
    }
  }
  return (
    <span className={styles.editor}>
      <input
        autoFocus
        // Selected, so typing replaces the value and arrow keys still allow small fixes.
        onFocus={(event) => event.target.select()}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        onBlur={finish}
      />
    </span>
  )
}
