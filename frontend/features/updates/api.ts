import { listen } from '@tauri-apps/api/event'
import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'

export type { Update }

export const updatesApi = {
  /** The newer version published, or `null` when this one is the latest. */
  check: () => check(),
  /** Downloads and installs `update`, telling how many bytes came of how many (`null` when unknown). */
  install: (update: Update, onProgress: (downloaded: number, total: number | null) => void) => {
    let downloaded = 0
    let total: number | null = null
    return update.downloadAndInstall((event) => {
      if (event.event === 'Started') total = event.data.contentLength ?? null
      if (event.event === 'Progress') downloaded += event.data.chunkLength
      onProgress(downloaded, total)
    })
  },
  /** Starts the installed version in place of this one. */
  relaunch: () => relaunch(),
  /** The app menu's "Check for Updates…"; resolves to the function that stops listening. */
  onMenuCheck: (handler: () => void) => listen('check-for-updates', handler),
}
