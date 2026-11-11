import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import type { TableRef } from '../../domain/table'
import { tableDataApi, type RowsPage } from './api'
import type { RowFilter } from './rowFilter'

/** Rows per page the footer offers; the backend reads at most 1000. */
export const PAGE_SIZES = [50, 100, 200, 500, 1000]
export const DEFAULT_PAGE_SIZE = 200

type Result = { key: string } & ({ page: RowsPage } | { error: string })

/**
 * Loads one page of rows at a time; a new filter or page size starts again at
 * the first page. A new `revision` reads the page again: the rows changed
 * elsewhere, e.g. an import added some.
 */
export function useRowsPage(
  connectionId: string,
  table: TableRef,
  filter: RowFilter,
  pageSize: number,
  revision = 0,
) {
  const filterKey = JSON.stringify([filter, pageSize])
  const [paging, setPaging] = useState({ filterKey, offset: 0 })
  const offset = paging.filterKey === filterKey ? paging.offset : 0
  const setOffset = (next: (current: number) => number) => setPaging({ filterKey, offset: next(offset) })
  const [reloads, setReloads] = useState(0)
  const [result, setResult] = useState<Result | null>(null)

  // Identifies the request; a result for any other key is stale.
  const key = JSON.stringify([connectionId, table, filter, pageSize, offset, reloads, revision])

  useEffect(() => {
    // Ignore the response if the request changed before it arrived.
    const request = { cancelled: false }
    tableDataApi
      .fetchRows(connectionId, table, filter, offset, pageSize)
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
    previous: () => setOffset((current) => Math.max(0, current - pageSize)),
    next: () => setOffset((current) => current + pageSize),
    reload: () => setReloads((current) => current + 1),
  }
}
