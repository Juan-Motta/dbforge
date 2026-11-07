import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import type { TableRef } from '../../domain/table'
import { tableDataApi, type RowsPage } from './api'
import type { RowFilter } from './rowFilter'

export const PAGE_SIZE = 200

type Result = { key: string } & ({ page: RowsPage } | { error: string })

/** Loads one page of rows at a time; a new filter starts again at the first page. */
export function useRowsPage(connectionId: string, table: TableRef, filter: RowFilter) {
  const filterKey = JSON.stringify(filter)
  const [paging, setPaging] = useState({ filterKey, offset: 0 })
  const offset = paging.filterKey === filterKey ? paging.offset : 0
  const setOffset = (next: (current: number) => number) => setPaging({ filterKey, offset: next(offset) })
  const [reloads, setReloads] = useState(0)
  const [result, setResult] = useState<Result | null>(null)

  // Identifies the request; a result for any other key is stale.
  const key = JSON.stringify([connectionId, table, filter, offset, reloads])

  useEffect(() => {
    // Ignore the response if the request changed before it arrived.
    const request = { cancelled: false }
    tableDataApi
      .fetchRows(connectionId, table, filter, offset, PAGE_SIZE)
      .then((page) => !request.cancelled && setResult({ key, page }))
      .catch((error) => !request.cancelled && setResult({ key, error: errorMessage(error) }))
    return () => {
      request.cancelled = true
    }
    // `key` covers the filter by value; the object itself may be new each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  const loading = result?.key !== key
  // Keep showing the previous page while the next one loads.
  const page = result && 'page' in result ? result.page : null
  const error = !loading && result && 'error' in result ? result.error : null

  return {
    page,
    error,
    loading,
    offset,
    hasPrevious: offset > 0,
    hasNext: page?.hasMore ?? false,
    previous: () => setOffset((current) => Math.max(0, current - PAGE_SIZE)),
    next: () => setOffset((current) => current + PAGE_SIZE),
    reload: () => setReloads((current) => current + 1),
  }
}
