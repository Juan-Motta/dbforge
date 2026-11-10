import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Icon, type IconName } from '../../components/Icon'
import type { TableRef } from '../../domain/table'
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import { parseQuery, rank, splitAtMatch, type Match } from './search'
import { useRelations } from './useRelations'
import styles from './CommandPalette.module.css'

/** Most databases fit; past this, typing narrows the list. */
const MAX_TABLES = 50

export interface Command {
  id: string
  label: string
  detail?: string
  run: () => void
}

/** Where tables are searched: a connection and, optionally, one of its databases. */
export interface PaletteSource {
  connectionId: string
  connectionName: string
  database?: string
}

interface CommandPaletteProps {
  source?: PaletteSource
  commands: Command[]
  onOpenTable: (connectionId: string, table: TableRef) => void
  onClose: () => void
}

interface Entry {
  id: string
  label: string
  detail?: string
  icon: IconName
  run: () => void
}

type Ranked = { item: Entry; match: Match }

/** ⌘K: jump to a table or view of the current connection, or run a command. */
export function CommandPalette({ source, commands, onOpenTable, onClose }: CommandPaletteProps) {
  const m = useMessages(messages)
  const dialog = useRef<HTMLDialogElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const relations = useRelations(source?.connectionId, source?.database)

  useEffect(() => dialog.current?.showModal(), [])

  const { text, commandsOnly } = parseQuery(query)
  const tableEntries: Entry[] =
    source && relations.list
      ? relations.list.relations.map((relation) => ({
          id: `table:${relation.schema}.${relation.name}`,
          label: relation.name,
          detail: `${source.connectionName} · ${relations.list!.database}.${relation.schema}`,
          icon: relation.kind,
          run: () =>
            onOpenTable(source.connectionId, {
              database: relations.list!.database,
              schema: relation.schema,
              name: relation.name,
            }),
        }))
      : []
  const commandEntries: Entry[] = commands.map((command) => ({
    ...command,
    id: `command:${command.id}`,
    icon: 'chevronRight',
  }))

  const tables = commandsOnly ? [] : rank(tableEntries, text, MAX_TABLES)
  const commandResults = rank(commandEntries, text)
  const results = [...tables, ...commandResults]
  const current = Math.min(active, results.length - 1)

  // Keep the highlighted row visible while moving with the arrow keys.
  useEffect(() => {
    list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [current])

  const run = (entry: Entry) => {
    onClose()
    entry.run()
  }

  const onKeyDown = (event: KeyboardEvent) => {
    const moves: Record<string, number> = { ArrowDown: 1, ArrowUp: -1 }
    if (event.key in moves && results.length > 0) {
      event.preventDefault()
      setActive((current + moves[event.key] + results.length) % results.length)
    } else if (event.key === 'Enter' && results[current]) {
      event.preventDefault()
      run(results[current].item)
    }
  }

  const tableStatus = commandsOnly
    ? null
    : !source
      ? m.selectConnection
      : relations.loading
        ? m.loadingTables
        : relations.error
          ? m.loadTablesFailed(relations.error)
          : tables.length === 0
            ? m.noTables
            : null

  const renderGroup = (title: string, items: Ranked[], offset: number, status: string | null) => (
    <div role="group" aria-label={title}>
      <div className={styles.groupTitle}>{title}</div>
      {status && <p className={styles.status}>{status}</p>}
      {items.map(({ item, match }, i) => {
        const [before, hit, after] = splitAtMatch(item.label, match)
        const index = offset + i
        return (
          <div
            key={item.id}
            id={`palette-${item.id}`}
            role="option"
            aria-selected={index === current}
            className={styles.item}
            onMouseMove={() => setActive(index)}
            onClick={() => run(item)}
          >
            <Icon name={item.icon} className={styles.icon} />
            <span className={styles.label}>
              {before}
              <mark>{hit}</mark>
              {after}
            </span>
            {item.detail && <span className={styles.detail}>{item.detail}</span>}
          </div>
        )
      })}
    </div>
  )

  return (
    <dialog
      ref={dialog}
      aria-label={m.palette}
      className={styles.palette}
      onCancel={(event) => {
        event.preventDefault()
        onClose()
      }}
      // A click on the backdrop lands on the dialog itself.
      onClick={(event) => event.target === dialog.current && onClose()}
    >
      <div className={styles.search}>
        <Icon name="search" size={18} className={styles.icon} />
        <input
          autoFocus
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-results"
          aria-activedescendant={results[current] ? `palette-${results[current].item.id}` : undefined}
          aria-label={m.searchLabel}
          placeholder={source ? m.searchPlaceholder(source.connectionName) : m.typeCommand}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value)
            setActive(0)
          }}
          onKeyDown={onKeyDown}
        />
      </div>

      <div ref={list} id="palette-results" role="listbox" className={styles.results}>
        {!commandsOnly && renderGroup(m.tablesAndViews, tables, 0, tableStatus)}
        {/* An empty command list is noise unless commands are all that was asked for. */}
        {(commandsOnly || commandResults.length > 0) &&
          renderGroup(m.commands, commandResults, tables.length, commandResults.length ? null : m.noCommands)}
      </div>

      <footer className={styles.footer}>
        <span>
          <kbd>↑↓</kbd> {m.navigate}
        </span>
        <span>
          <kbd>↵</kbd> {m.open}
        </span>
        <span>
          <kbd>esc</kbd> {m.close}
        </span>
        <span className={styles.tip}>
          {m.typeBefore} <code>&gt;</code> {m.typeAfter}
        </span>
      </footer>
    </dialog>
  )
}
