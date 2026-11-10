import type { TableRef } from '../domain/table'
import type { RowFilter } from '../features/table-data/rowFilter'
import type { Section } from '../features/table-structure/sections'
import type { AppMessages } from './messages'

/** What a table's tab shows: its rows, its diagram, or a section of its structure. */
export type TableTabView = 'data' | 'diagram' | Section

export type Tab = { id: string; title: string; connectionId: string } & (
  | { kind: 'query'; database?: string; initialSql?: string; container?: string }
  | { kind: 'table'; table: TableRef; filter?: RowFilter; view?: TableTabView }
  | { kind: 'diagram'; database: string; schema: string }
  | { kind: 'keyspace'; database: string }
)

export interface TabsState {
  tabs: Tab[]
  activeId: string | null
  /** Numbers new query tabs: "Query 1", "Query 2", … */
  queriesCreated: number
}

export const NO_TABS: TabsState = { tabs: [], activeId: null, queriesCreated: 0 }

/** Opens a new query console, optionally prefilled with `initialSql` (on a Cosmos DB `container`). */
export function newQueryTab(
  state: TabsState,
  m: AppMessages,
  connectionId: string,
  database?: string,
  initialSql?: string,
  container?: string,
): TabsState {
  const n = state.queriesCreated + 1
  const tab: Tab = {
    id: `query:${n}`,
    kind: 'query',
    title: m.queryTab(n),
    connectionId,
    database,
    initialSql,
    container,
  }
  return { ...openTab(state, tab), queriesCreated: n }
}

/** Opens a table, or switches to it when it is already open. */
export function openTableTab(
  state: TabsState,
  connectionId: string,
  table: TableRef,
  /** Opens the table already filtered, in a tab of its own (e.g. following a foreign key). */
  filter?: RowFilter,
  /** Shows this view, also when the tab was already open. */
  view?: TableTabView,
): TabsState {
  const base = `table:${connectionId}:${table.database}.${table.schema}.${table.name}`
  const tab: Tab = filter
    ? {
        id: `${base}?${filter.filter}`,
        kind: 'table',
        title: `${table.name} · ${filter.filter}`,
        connectionId,
        table,
        filter,
      }
    : { id: base, kind: 'table', title: table.name, connectionId, table }
  const opened = openTab(state, tab)
  return view ? setTableView(opened, tab.id, view) : opened
}

/** Switches a table's tab to `view`. */
export function setTableView(state: TabsState, id: string, view: TableTabView): TabsState {
  return {
    ...state,
    tabs: state.tabs.map((tab) => (tab.id === id && tab.kind === 'table' ? { ...tab, view } : tab)),
  }
}

/** Opens a schema's ER diagram, or switches to it when it is already open. */
export function openDiagramTab(
  state: TabsState,
  m: AppMessages,
  connectionId: string,
  database: string,
  schema: string,
): TabsState {
  const id = `diagram:${connectionId}:${database}.${schema}`
  return openTab(state, { id, kind: 'diagram', title: m.diagramTab(schema), connectionId, database, schema })
}

/** Opens a Redis database in the key browser, or switches to it when it is already open. */
export function openKeyspaceTab(
  state: TabsState,
  connectionId: string,
  connectionName: string,
  database: string,
): TabsState {
  const id = `keyspace:${connectionId}:${database}`
  return openTab(state, {
    id,
    kind: 'keyspace',
    title: `${connectionName} · ${database}`,
    connectionId,
    database,
  })
}

function openTab(state: TabsState, tab: Tab): TabsState {
  const exists = state.tabs.some((t) => t.id === tab.id)
  return { ...state, tabs: exists ? state.tabs : [...state.tabs, tab], activeId: tab.id }
}

/** Closes a tab; when it was active, its right neighbor (or else left) becomes active. */
export function closeTab(state: TabsState, id: string): TabsState {
  const index = state.tabs.findIndex((t) => t.id === id)
  if (index < 0) return state
  const tabs = state.tabs.filter((t) => t.id !== id)
  const activeId =
    state.activeId === id ? (tabs[Math.min(index, tabs.length - 1)]?.id ?? null) : state.activeId
  return { ...state, tabs, activeId }
}

/** Closes every tab of a connection, e.g. after it is deleted. */
export function closeConnectionTabs(state: TabsState, connectionId: string): TabsState {
  return state.tabs
    .filter((t) => t.connectionId === connectionId)
    .reduce((current, tab) => closeTab(current, tab.id), state)
}
