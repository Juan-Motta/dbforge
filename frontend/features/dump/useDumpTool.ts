import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { dumpApi, type DumpTool, type ToolInfo } from './api'

type State = { state: 'loading' } | { state: 'found'; tool: ToolInfo } | { state: 'missing'; error: string }

/** The pg_dump or psql DBForge will run, and changing it. */
export function useDumpTool(name: DumpTool) {
  const [state, setState] = useState<State>({ state: 'loading' })
  // A path that couldn't be used: said once, next to the tool still in use.
  const [refused, setRefused] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    dumpApi
      .tool(name)
      .then((tool) => current && setState({ state: 'found', tool }))
      .catch((error) => current && setState({ state: 'missing', error: errorMessage(error) }))
    return () => {
      current = false
    }
  }, [name])

  const change = async (path: string | null) => {
    try {
      setState({ state: 'found', tool: await dumpApi.setTool(name, path) })
      setRefused(null)
    } catch (error) {
      setRefused(errorMessage(error))
    }
  }

  return { ...state, refused, change }
}
