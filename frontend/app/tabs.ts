import type { TableRef } from '../domain/table'

export type Tab = { id: string; title: string; connectionId: string } & (
  | { kind: 'query'; database?: string; initialSql?: string }
  | { kind: 'table'; table: TableRef }
  | { kind: 'diagram'; database: string; schema: string }
)

export interface TabsState {
  tabs: Tab[]
  activeId: string | null
  /** Numbers new query tabs: "Query 1", "Query 2", … */
  queriesCreated: number
}

export const NO_TABS: TabsState = { tabs: [], activeId: null, queriesCreated: 0 }

/** Opens a new query console, optionally prefilled with `initialSql`. */
export function newQueryTab(
  state: TabsState,
  connectionId: string,
  database?: string,
  initialSql?: string,
): TabsState {
  const n = state.queriesCreated + 1
  const tab: Tab = {
    id: `query:${n}`,
    kind: 'query',
    title: `Query ${n}`,
    connectionId,
    database,
    initialSql,
  }
  return { ...openTab(state, tab), queriesCreated: n }
}

/** Opens a table, or switches to it when it is already open. */
export function openTableTab(state: TabsState, connectionId: string, table: TableRef): TabsState {
  const id = `table:${connectionId}:${table.database}.${table.schema}.${table.name}`
  return openTab(state, { id, kind: 'table', title: table.name, connectionId, table })
}

/** Opens a schema's ER diagram, or switches to it when it is already open. */
export function openDiagramTab(
  state: TabsState,
  connectionId: string,
  database: string,
  schema: string,
): TabsState {
  const id = `diagram:${connectionId}:${database}.${schema}`
  return openTab(state, { id, kind: 'diagram', title: `${schema} diagram`, connectionId, database, schema })
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
