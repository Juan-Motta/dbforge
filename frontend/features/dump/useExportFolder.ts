import { useEffect, useState } from 'react'
import { documentDir } from '@tauri-apps/api/path'
import { defaultExportFolder, loadExportFolder, saveExportFolder } from './exportFolder'

/** Where exports go: the folder chosen in the settings, else DBForge in Documents. */
export function useExportFolder() {
  const [chosen, setChosen] = useState(loadExportFolder)
  const [fallback, setFallback] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    documentDir()
      .then((documents) => current && setFallback(defaultExportFolder(documents)))
      .catch(() => {})
    return () => {
      current = false
    }
  }, [])

  return {
    /** `null` while the default is being looked up. */
    folder: chosen ?? fallback,
    /** Whether the user chose it rather than taking the default. */
    custom: chosen !== null,
    choose: (folder: string | null) => {
      saveExportFolder(folder)
      setChosen(folder)
    },
  }
}
