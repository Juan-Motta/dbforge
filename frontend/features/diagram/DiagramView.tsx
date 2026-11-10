import { memo, useCallback, useEffect, useMemo, useRef, useState, type PointerEvent } from 'react'
import { Button, IconButton } from '../../components/Button'
import { shortTypeName } from '../../components/data-grid/columns'
import { Icon } from '../../components/Icon'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { SchemaDiagram, DiagramTable } from './api'
import {
  CARD_WIDTH,
  cardHeight,
  centerOn,
  defaultSelection,
  fitView,
  layout,
  localRelationships,
  neighborhood,
  relationshipPath,
  shownColumns,
  zoomAt,
  type ColumnMode,
  type Point,
  type View,
} from './layout'
import { messages } from './messages'
import { useSchemaDiagram } from './useSchemaDiagram'
import styles from './DiagramView.module.css'

interface DiagramViewProps {
  connectionId: string
  database: string
  schema: string
  /** A table's own diagram: centred on it, with its neighbours, and no table list. */
  focus?: string
  /** Opens the whole schema's diagram, from a table's own one. */
  onOpenSchema?: () => void
}

/** The ER diagram of one schema, or of one table and its neighbours. */
export function DiagramView({ connectionId, database, schema, focus, onOpenSchema }: DiagramViewProps) {
  const m = useMessages(messages)
  const { diagram, error, reload } = useSchemaDiagram(connectionId, database, schema)
  if (error) return <p className={styles.message + ' ' + styles.error}>{error}</p>
  if (!diagram) return <p className={styles.message}>{m.loading(schema)}</p>
  if (diagram.tables.length === 0) return <p className={styles.message}>{m.noTables(schema)}</p>
  if (focus && !diagram.tables.some((t) => t.name === focus)) {
    return <p className={styles.message}>{m.isView(focus)}</p>
  }
  return (
    <Diagram
      diagram={diagram}
      schema={schema}
      database={database}
      focus={focus}
      onOpenSchema={onOpenSchema}
      onReload={reload}
    />
  )
}

/** How far a table's diagram follows relations; `null` shows the whole schema. */
const REACH: { label: 'direct' | 'twoSteps' | 'wholeSchema'; steps: number | null }[] = [
  { label: 'direct', steps: 1 },
  { label: 'twoSteps', steps: 2 },
  { label: 'wholeSchema', steps: null },
]

/** What a pointer drag is moving: the whole canvas, or one card. */
type Drag =
  | { kind: 'pan'; start: Point; origin: View; moved: boolean }
  | { kind: 'card'; name: string; start: Point; origin: Point; moved: boolean }

interface DiagramProps {
  diagram: SchemaDiagram
  schema: string
  database: string
  focus?: string
  onOpenSchema?: () => void
  onReload: () => void
}

function Diagram({ diagram, schema, database, focus, onOpenSchema, onReload }: DiagramProps) {
  const m = useMessages(messages)
  const relationships = useMemo(() => localRelationships(diagram, schema), [diagram, schema])
  // Every foreign key column gets a mark, even when its target is in another schema.
  const foreignKeys = useMemo(
    () => new Set(diagram.relationships.flatMap((r) => r.columns.map((c) => `${r.table}.${c}`))),
    [diagram],
  )
  const [reach, setReach] = useState<number | null>(1)
  const [visible, setVisible] = useState(() =>
    focus ? neighborhood(relationships, focus, 1) : defaultSelection(diagram, relationships),
  )
  const [mode, setMode] = useState<ColumnMode>('all')
  const [filter, setFilter] = useState('')
  const [moved, setMoved] = useState<Map<string, Point>>(new Map())
  const [view, setView] = useState<View>({ x: 40, y: 40, scale: 1 })
  /** Pinned with a click; stays highlighted when the pointer moves away. */
  const [pinned, setPinned] = useState<string | null>(focus ?? null)
  const [hovered, setHovered] = useState<string | null>(null)
  // Pointer capture makes the card "lose" the hover mid-drag, so the dragged one is tracked too.
  const [dragging, setDragging] = useState<string | null>(null)
  // Hovering shows a table's relations at a glance; a pin keeps them while exploring.
  const active = dragging ?? hovered ?? pinned
  const canvas = useRef<HTMLDivElement | null>(null)
  const drag = useRef<Drag | null>(null)

  const cards = useMemo(
    () => buildCards(diagram, visible, mode, foreignKeys),
    [diagram, visible, mode, foreignKeys],
  )
  const autoPositions = useMemo(
    () =>
      layout(
        cards.map((c) => ({ name: c.table.name, height: c.height })),
        relationships,
      ),
    [cards, relationships],
  )
  const positionOf = (name: string) => moved.get(name) ?? autoPositions.get(name)!
  const byName = useMemo(() => new Map(cards.map((c) => [c.table.name, c])), [cards])
  const lines = useMemo(
    () => relationships.filter((r) => byName.has(r.table) && byName.has(r.referencedTable)),
    [relationships, byName],
  )
  // The tables tied to the active one, worked out once rather than for each card.
  const related = useMemo(() => {
    const names = new Set<string>()
    if (active === null) return names
    names.add(active)
    for (const r of lines) {
      if (r.table === active) names.add(r.referencedTable)
      if (r.referencedTable === active) names.add(r.table)
    }
    return names
  }, [lines, active])
  const hover = useCallback(
    (name: string, inside: boolean) =>
      setHovered((current) => (inside ? name : current === name ? null : current)),
    [],
  )

  const fit = () => {
    const box = canvas.current?.getBoundingClientRect()
    if (!box) return
    setView(
      fitView(
        cards.map((c) => ({ position: positionOf(c.table.name), height: c.height })),
        box,
      ),
    )
  }

  // Fit once when the canvas first appears, so a big schema starts fully in view.
  const fitted = useRef(false)
  const attachCanvas = useCallback(
    (node: HTMLDivElement | null) => {
      canvas.current = node
      if (!node || fitted.current) return
      fitted.current = true
      const box = node.getBoundingClientRect()
      setView(
        fitView(
          cards.map((c) => ({ position: autoPositions.get(c.table.name)!, height: c.height })),
          box,
        ),
      )
    },
    // Only the first attachment matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  )

  // The wheel (and pinch) zooms around the cursor; dragging is how the canvas moves.
  // React's own wheel listener is passive, so it could not stop the page from scrolling.
  useEffect(() => {
    const node = canvas.current
    if (!node) return
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      const box = node.getBoundingClientRect()
      const anchor = { x: event.clientX - box.left, y: event.clientY - box.top }
      setView((current) => zoomAt(current, Math.exp(-event.deltaY / 300), anchor))
    }
    node.addEventListener('wheel', onWheel, { passive: false })
    return () => node.removeEventListener('wheel', onWheel)
  }, [])

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    // Anywhere on a card moves that card; anywhere else pans the canvas.
    const card = (event.target as HTMLElement).closest<HTMLElement>('[data-card]')
    const start = { x: event.clientX, y: event.clientY }
    drag.current = card
      ? {
          kind: 'card',
          name: card.dataset.card!,
          start,
          origin: positionOf(card.dataset.card!),
          moved: false,
        }
      : { kind: 'pan', start, origin: view, moved: false }
    setDragging(card?.dataset.card ?? null)
    event.currentTarget.setPointerCapture(event.pointerId)
  }

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current
    if (!current) return
    const dx = event.clientX - current.start.x
    const dy = event.clientY - current.start.y
    if (current.kind === 'pan') {
      current.moved ||= Math.hypot(dx, dy) > 3
      setView({ ...current.origin, x: current.origin.x + dx, y: current.origin.y + dy })
    } else if (current.moved || Math.hypot(dx, dy) > 3) {
      current.moved = true
      const position = { x: current.origin.x + dx / view.scale, y: current.origin.y + dy / view.scale }
      setMoved((previous) => new Map(previous).set(current.name, position))
    }
  }

  // The canvas captures the pointer while dragging, so the browser's dblclick lands
  // on the canvas rather than the card; double clicks are detected here instead.
  const lastClick = useRef<{ name: string; time: number } | null>(null)

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current
    drag.current = null
    setDragging(null)
    // A click on empty canvas (not a pan) clears the pinned highlight.
    if (current?.kind === 'pan' && !current.moved) {
      setPinned(null)
      lastClick.current = null
    }
    if (current?.kind !== 'card' || current.moved) return
    const previous = lastClick.current
    if (previous?.name === current.name && event.timeStamp - previous.time < 400) {
      // Double click: bring the table close enough to read, and keep it highlighted.
      lastClick.current = null
      const box = canvas.current?.getBoundingClientRect()
      const card = byName.get(current.name)
      if (box && card) setView(centerOn(positionOf(current.name), card.height, box))
      setPinned(current.name)
      return
    }
    lastClick.current = { name: current.name, time: event.timeStamp }
    // A single click pins the card's highlight, or unpins it.
    setPinned((name) => (name === current.name ? null : current.name))
  }

  // A table's diagram refits whenever its reach changes, once the new cards are laid out.
  const refit = useRef(false)
  useEffect(() => {
    if (!refit.current) return
    refit.current = false
    fit()
  })
  const changeReach = (steps: number | null) => {
    setReach(steps)
    setVisible(
      steps === null
        ? new Set(diagram.tables.map((t) => t.name))
        : neighborhood(relationships, focus!, steps),
    )
    setMoved(new Map())
    refit.current = true
  }

  const zoomBy = (factor: number) => {
    const box = canvas.current?.getBoundingClientRect()
    if (box) setView((current) => zoomAt(current, factor, { x: box.width / 2, y: box.height / 2 }))
  }

  const toggle = (name: string) =>
    setVisible((previous) => {
      const next = new Set(previous)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })

  const listed = diagram.tables.filter((t) => t.name.toLowerCase().includes(filter.toLowerCase()))

  return (
    <div className={styles.diagram}>
      {!focus && (
        <aside className={styles.sidebar} aria-label={m.tables}>
          <div className={styles.sidebarHeader}>
            <span className={styles.title}>
              {database}.{schema}
            </span>
            <span className="muted">{m.shownOf(visible.size, diagram.tables.length)}</span>
          </div>
          <input
            type="search"
            className={styles.filter}
            aria-label={m.filter}
            placeholder={m.filterPlaceholder}
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
          />
          <div className={styles.bulk}>
            <button
              type="button"
              onClick={() => setVisible(new Set([...visible, ...listed.map((t) => t.name)]))}
            >
              {m.showAll}
            </button>
            <button
              type="button"
              onClick={() =>
                setVisible(new Set([...visible].filter((n) => !listed.some((t) => t.name === n))))
              }
            >
              {m.hideAll}
            </button>
          </div>
          <div className={styles.tables}>
            {listed.map((table) => (
              <label key={table.name} className={styles.tableOption}>
                <input
                  type="checkbox"
                  checked={visible.has(table.name)}
                  onChange={() => toggle(table.name)}
                />
                <Icon name="table" size={14} />
                <span>{table.name}</span>
              </label>
            ))}
          </div>
        </aside>
      )}

      <div
        ref={attachCanvas}
        className={styles.canvas}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      >
        <div
          className={styles.world}
          style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})` }}
        >
          <Lines lines={lines} cards={byName} moved={moved} autoPositions={autoPositions} active={active} />
          {cards.map(({ table, columns }) => {
            const { x, y } = positionOf(table.name)
            return (
              <TableCard
                key={table.name}
                table={table}
                columns={columns}
                x={x}
                y={y}
                foreignKeys={foreignKeys}
                focused={active === table.name}
                pinned={pinned === table.name}
                dimmed={active !== null && !related.has(table.name)}
                onHover={hover}
              />
            )
          })}
        </div>

        <div className={styles.toolbar} onPointerDown={(event) => event.stopPropagation()}>
          {focus && (
            <div className={styles.segmented} role="group" aria-label={m.relationsFollowed}>
              {REACH.map((option) => (
                <button
                  key={option.label}
                  type="button"
                  aria-pressed={reach === option.steps}
                  onClick={() => changeReach(option.steps)}
                >
                  {m[option.label]}
                </button>
              ))}
            </div>
          )}
          <div className={styles.segmented} role="group" aria-label={m.columnsShown}>
            <button type="button" aria-pressed={mode === 'all'} onClick={() => setMode('all')}>
              {m.columns}
            </button>
            <button type="button" aria-pressed={mode === 'keys'} onClick={() => setMode('keys')}>
              {m.keysOnly}
            </button>
          </div>
          <Button onClick={() => setMoved(new Map())}>{m.autoLayout}</Button>
          <IconButton label={m.reload} onClick={onReload}>
            <Icon name="refresh" />
          </IconButton>
          {onOpenSchema && <Button onClick={onOpenSchema}>{m.openSchema}</Button>}
        </div>

        <div className={styles.zoom} onPointerDown={(event) => event.stopPropagation()}>
          <IconButton label={m.zoomOut} onClick={() => zoomBy(1 / 1.2)}>
            <Icon name="minus" />
          </IconButton>
          <span className={styles.percent}>{Math.round(view.scale * 100)}%</span>
          <IconButton label={m.zoomIn} onClick={() => zoomBy(1.2)}>
            <Icon name="plus" />
          </IconButton>
          <Button onClick={fit}>{m.fit}</Button>
        </div>
      </div>
    </div>
  )
}

/** The cards to draw: the chosen tables with the columns the mode shows. */
function buildCards(diagram: SchemaDiagram, names: Set<string>, mode: ColumnMode, foreignKeys: Set<string>) {
  return diagram.tables
    .filter((t) => names.has(t.name))
    .map((table) => {
      const columns = shownColumns(table, mode, foreignKeys)
      return { table, columns, height: cardHeight(columns.length) }
    })
}

type Card = ReturnType<typeof buildCards>[number]

interface LinesProps {
  lines: SchemaDiagram['relationships']
  cards: Map<string, Card>
  moved: Map<string, Point>
  autoPositions: Map<string, Point>
  active: string | null
}

/**
 * The relationships between the cards shown. Drawn again only when a card
 * moves or another table becomes active: panning and zooming move them as a whole.
 */
const Lines = memo(function Lines({ lines, cards, moved, autoPositions, active }: LinesProps) {
  const positionOf = (name: string) => moved.get(name) ?? autoPositions.get(name)!
  return (
    <svg className={styles.lines} aria-hidden="true">
      {lines.map((r) => {
        const from = cards.get(r.table)!
        const to = cards.get(r.referencedTable)!
        const highlighted = active !== null && (r.table === active || r.referencedTable === active)
        return (
          // A constraint's name is unique within its table only.
          <path
            key={`${r.table}.${r.name}`}
            d={relationshipPath(
              positionOf(r.table),
              from.columns.findIndex((c) => c.name === r.columns[0]),
              positionOf(r.referencedTable),
              to.columns.findIndex((c) => c.name === r.referencedColumns[0]),
            )}
            data-highlighted={highlighted || undefined}
            data-dimmed={(active !== null && !highlighted) || undefined}
          >
            <title>
              {r.name}: {r.table}({r.columns.join(', ')}) → {r.referencedTable}(
              {r.referencedColumns.join(', ')})
            </title>
          </path>
        )
      })}
    </svg>
  )
})

interface TableCardProps {
  table: DiagramTable
  columns: Card['columns']
  x: number
  y: number
  foreignKeys: Set<string>
  focused: boolean
  pinned: boolean
  dimmed: boolean
  /** The same function between renders, so a card is drawn again only when it changes. */
  onHover: (name: string, inside: boolean) => void
}

/** One table. Drawn again only when it moves or its highlight changes, not when the canvas pans. */
const TableCard = memo(function TableCard({
  table,
  columns,
  x,
  y,
  foreignKeys,
  focused,
  pinned,
  dimmed,
  onHover,
}: TableCardProps) {
  return (
    <div
      className={styles.card}
      style={{ left: x, top: y, width: CARD_WIDTH }}
      data-card={table.name}
      data-focused={focused || undefined}
      data-pinned={pinned || undefined}
      data-dimmed={dimmed || undefined}
      onPointerEnter={() => onHover(table.name, true)}
      onPointerLeave={() => onHover(table.name, false)}
    >
      <div className={styles.cardHeader}>
        <Icon name="table" size={14} />
        <span className={styles.cardName}>{table.name}</span>
        {table.rowEstimate !== null && (
          <span className={styles.rows}>{COMPACT.format(table.rowEstimate)}</span>
        )}
      </div>
      {columns.map((column) => {
        const fk = foreignKeys.has(`${table.name}.${column.name}`)
        return (
          <div key={column.name} className={styles.column}>
            <span className={styles.key} data-kind={column.primaryKey ? 'pk' : fk ? 'fk' : undefined}>
              {column.primaryKey ? 'PK' : fk ? 'FK' : ''}
            </span>
            <span className={styles.columnName}>{column.name}</span>
            <span className={styles.columnType}>{shortTypeName(column.dataType)}</span>
          </div>
        )
      })}
    </div>
  )
})

/** Row estimates as `12K`, `3.4M`. */
const COMPACT = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })
