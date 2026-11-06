import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { tableDataApi, type RowsPage, type TableRef } from './api'

export const PAGE_SIZE = 200

type Result = { key: string } & ({ page: RowsPage } | { error: string })

/** Loads one page of rows at a time. Mount it per table so paging starts at 0. */
export function useRowsPage(connectionId: string, table: TableRef) {
  const [offset, setOffset] = useState(0)
  const [reloads, setReloads] = useState(0)
  const [result, setResult] = useState<Result | null>(null)

  // Identifies the request; a result for any other key is stale.
  const key = JSON.stringify([connectionId, table, offset, reloads])

  useEffect(() => {
    // Ignore the response if the request changed before it arrived.
    const request = { cancelled: false }
    tableDataApi
      .fetchRows(connectionId, table, offset, PAGE_SIZE)
      .then((page) => !request.cancelled && setResult({ key, page }))
      .catch((error) => !request.cancelled && setResult({ key, error: errorMessage(error) }))
    return () => {
      request.cancelled = true
    }
  }, [connectionId, table, offset, reloads, key])

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
