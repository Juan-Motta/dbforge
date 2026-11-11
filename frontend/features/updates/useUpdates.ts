import { useCallback, useEffect, useRef, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { updatesApi, type Update } from './api'
import { checkDue, lastCheck, noteCheck } from './schedule'

export type UpdateState =
  | { state: 'idle' }
  /** Only a check the user asked for shows while it runs. */
  | { state: 'checking' }
  | { state: 'available'; update: Update }
  | { state: 'upToDate' }
  | { state: 'installing'; update: Update; downloaded: number; total: number | null }
  | { state: 'failed'; error: string; update: Update | null }

/** How often to see whether the daily check is due, while the app stays open. */
const LOOK_EVERY = 60 * 60 * 1000

/**
 * Finds and installs new versions of DBForge. While `automatic`, it checks
 * once a day (at launch when a day has passed) and says something only when
 * there is an update; `check` is the user asking, from the menu or Settings,
 * and always gets an answer.
 */
export function useUpdates(automatic: boolean) {
  const [state, setState] = useState<UpdateState>({ state: 'idle' })
  const busy = useRef(false)

  const run = useCallback(async (manual: boolean) => {
    if (busy.current) return
    busy.current = true
    if (manual) setState({ state: 'checking' })
    try {
      const update = await updatesApi.check()
      noteCheck(Date.now())
      if (update) setState({ state: 'available', update })
      else setState(manual ? { state: 'upToDate' } : { state: 'idle' })
    } catch (error) {
      // An automatic check that fails (offline, say) stays quiet; the next one tries again.
      setState(manual ? { state: 'failed', error: errorMessage(error), update: null } : { state: 'idle' })
    } finally {
      busy.current = false
    }
  }, [])

  useEffect(() => {
    if (!automatic) return
    const look = () => {
      if (checkDue(lastCheck(), Date.now())) void run(false)
    }
    // A moment after launch, so the check doesn't compete with the first screen.
    const first = setTimeout(look, 5000)
    const timer = setInterval(look, LOOK_EVERY)
    return () => {
      clearTimeout(first)
      clearInterval(timer)
    }
  }, [automatic, run])

  useEffect(() => {
    const stop = updatesApi.onMenuCheck(() => void run(true))
    return () => void stop.then((unlisten) => unlisten())
  }, [run])

  const install = async (update: Update) => {
    setState({ state: 'installing', update, downloaded: 0, total: null })
    try {
      await updatesApi.install(update, (downloaded, total) =>
        setState({ state: 'installing', update, downloaded, total }),
      )
      await updatesApi.relaunch()
    } catch (error) {
      setState({ state: 'failed', error: errorMessage(error), update })
    }
  }

  return {
    state,
    check: () => void run(true),
    install: (update: Update) => void install(update),
    dismiss: () => setState({ state: 'idle' }),
  }
}
