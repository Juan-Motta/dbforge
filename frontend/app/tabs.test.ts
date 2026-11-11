import { describe, expect, it } from 'vitest'
import {
  closeConnectionTabs,
  closeDatabaseTabs,
  closeTab,
  newQueryTab,
  NO_TABS,
  openDiagramTab,
  openKeyspaceTab,
  openTableTab,
} from './tabs'
import { messages } from './messages'

const en = messages.en

const orders = { database: 'shop', schema: 'public', name: 'orders' }
const users = { database: 'shop', schema: 'public', name: 'users' }

const titles = (state: { tabs: { title: string }[] }) => state.tabs.map((t) => t.title)

describe('tabs', () => {
  it('numbers query tabs and activates the new one', () => {
    const state = newQueryTab(newQueryTab(NO_TABS, en, 'pg', 'shop'), en, 'pg')
    expect(titles(state)).toEqual(['Query 1', 'Query 2'])
    expect(state.activeId).toBe('query:2')
  })

  it('opens one key browser per Redis database', () => {
    let state = openKeyspaceTab(NO_TABS, 'cache', 'cache-redis', 'db0')
    state = openKeyspaceTab(state, 'cache', 'cache-redis', 'db1')
    state = openKeyspaceTab(state, 'cache', 'cache-redis', 'db0')
    expect(titles(state)).toEqual(['cache-redis · db0', 'cache-redis · db1'])
    expect(state.activeId).toBe('keyspace:cache:db0')
  })

  it('opens one diagram tab per schema', () => {
    let state = openDiagramTab(NO_TABS, en, 'pg', 'shop', 'public')
    state = openDiagramTab(state, en, 'pg', 'shop', 'billing')
    state = openDiagramTab(state, en, 'pg', 'shop', 'public')
    expect(titles(state)).toEqual(['public diagram', 'billing diagram'])
    expect(state.activeId).toBe('diagram:pg:shop.public')
  })

  it('titles query and diagram tabs in the interface language', () => {
    const state = openDiagramTab(newQueryTab(NO_TABS, messages.es, 'pg'), messages.es, 'pg', 'shop', 'public')
    expect(titles(state)).toEqual(['Consulta 1', 'Diagrama de public'])
  })

  it('can prefill a query tab, e.g. from the history', () => {
    const state = newQueryTab(NO_TABS, en, 'pg', 'shop', 'SELECT 1')
    expect(state.tabs[0]).toMatchObject({ kind: 'query', database: 'shop', initialSql: 'SELECT 1' })
  })

  it('opens a Cosmos DB console on a container', () => {
    const state = newQueryTab(NO_TABS, en, 'cosmos', 'shop', 'SELECT * FROM c', 'order-items')
    expect(state.tabs[0]).toMatchObject({ database: 'shop', container: 'order-items' })
  })

  it('keeps numbering after a query tab closes', () => {
    const one = newQueryTab(NO_TABS, en, 'pg')
    const state = newQueryTab(closeTab(one, 'query:1'), en, 'pg')
    expect(titles(state)).toEqual(['Query 2'])
  })

  it('reuses the tab of a table that is already open', () => {
    let state = openTableTab(NO_TABS, 'pg', orders)
    state = openTableTab(state, 'pg', users)
    state = openTableTab(state, 'pg', orders)
    expect(titles(state)).toEqual(['orders', 'users'])
    expect(state.activeId).toBe(state.tabs[0].id)
  })

  it('opens a filtered table in its own tab, reused for the same filter', () => {
    const one = { filter: `"id" = '877'`, order: '' }
    let state = openTableTab(NO_TABS, 'pg', orders)
    state = openTableTab(state, 'pg', orders, one)
    state = openTableTab(state, 'pg', orders, one)
    expect(titles(state)).toEqual(['orders', `orders · "id" = '877'`])
    const tab = state.tabs[1]
    expect(tab.kind === 'table' && tab.filter).toEqual(one)
  })

  it('treats the same table on another connection as a different tab', () => {
    const state = openTableTab(openTableTab(NO_TABS, 'pg', orders), 'other', orders)
    expect(state.tabs).toHaveLength(2)
  })

  it('activates the right neighbor, then the left, when the active tab closes', () => {
    let state = openTableTab(openTableTab(newQueryTab(NO_TABS, en, 'pg'), 'pg', orders), 'pg', users)
    state = closeTab({ ...state, activeId: state.tabs[1].id }, state.tabs[1].id)
    expect(state.activeId).toBe(state.tabs[1].id)
    state = closeTab(state, state.tabs[1].id)
    expect(state.activeId).toBe('query:1')
    expect(closeTab(state, 'query:1').activeId).toBeNull()
  })

  it('keeps the active tab when another one closes', () => {
    const state = openTableTab(newQueryTab(NO_TABS, en, 'pg'), 'pg', orders)
    expect(closeTab(state, 'query:1').activeId).toBe(state.activeId)
  })

  it('closes all tabs of a connection', () => {
    let state = newQueryTab(openTableTab(NO_TABS, 'pg', orders), en, 'other')
    state = closeConnectionTabs(state, 'pg')
    expect(titles(state)).toEqual(['Query 1'])
    expect(state.activeId).toBe('query:1')
  })

  it('closes the tabs showing a dropped database and keeps its consoles', () => {
    let state = openTableTab(NO_TABS, 'pg', orders)
    state = openDiagramTab(state, en, 'pg', 'shop', 'public')
    state = openTableTab(state, 'pg', { ...orders, database: 'archive' })
    state = openTableTab(state, 'other', orders)
    state = newQueryTab(state, en, 'pg', 'shop')
    state = closeDatabaseTabs(state, 'pg', 'shop')
    expect(state.tabs.map((t) => t.id)).toEqual([
      'table:pg:archive.public.orders',
      'table:other:shop.public.orders',
      'query:1',
    ])
    expect(state.activeId).toBe('query:1')
  })
})
