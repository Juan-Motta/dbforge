import { useEffect, useState } from 'react'
import type { TableRef } from '../../domain/table'
import { errorMessage } from '../../lib/errors'
import { structureApi, type TableStructure } from './api'

type Result = { key: string } & ({ structure: TableStructure } | { error: string })

/** Loads a table's structure, again on every `reload()`. */
export function useTableStructure(connectionId: string, table: TableRef) {
  const [reloads, setReloads] = useState(0)
  const [result, setResult] = useState<Result | null>(null)

  // Identifies the request; a result for any other key is stale.
  const key = JSON.stringify([connectionId, table, reloads])

  useEffect(() => {
    const request = { cancelled: false }
    structureApi
      .get(connectionId, table)
      .then((structure) => !request.cancelled && setResult({ key, structure }))
      .catch((error) => !request.cancelled && setResult({ key, error: errorMessage(error) }))
    return () => {
      request.cancelled = true
    }
  }, [connectionId, table, key])

  const loading = result?.key !== key
  return {
    // Keep showing the previous structure while reloading.
    structure: result && 'structure' in result ? result.structure : null,
    error: !loading && result && 'error' in result ? result.error : null,
    loading,
    reload: () => setReloads((current) => current + 1),
  }
}
