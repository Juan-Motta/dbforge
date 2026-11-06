import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { diagramApi, type SchemaDiagram } from './api'

type Result = { key: string } & ({ diagram: SchemaDiagram } | { error: string })

/** Loads a schema's tables and foreign keys, again on `reload()`. */
export function useSchemaDiagram(connectionId: string, database: string, schema: string) {
  const [reloads, setReloads] = useState(0)
  const [result, setResult] = useState<Result | null>(null)
  const key = JSON.stringify([connectionId, database, schema, reloads])

  useEffect(() => {
    const request = { cancelled: false }
    diagramApi
      .load(connectionId, database, schema)
      .then((diagram) => !request.cancelled && setResult({ key, diagram }))
      .catch((error) => !request.cancelled && setResult({ key, error: errorMessage(error) }))
    return () => {
      request.cancelled = true
    }
  }, [connectionId, database, schema, key])

  const current = result?.key === key ? result : null
  return {
    diagram: current && 'diagram' in current ? current.diagram : null,
    error: current && 'error' in current ? current.error : null,
    reload: () => setReloads((n) => n + 1),
  }
}
