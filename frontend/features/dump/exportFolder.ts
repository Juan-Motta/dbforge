const STORAGE_KEY = 'dbforge.exportFolder'

/** The folder exports go to, as the settings chose it; `null` until one is chosen. */
export function loadExportFolder(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY)
  } catch {
    return null
  }
}

/** Keeps `folder` as where exports go; `null` goes back to the default one. */
export function saveExportFolder(folder: string | null) {
  try {
    if (folder) localStorage.setItem(STORAGE_KEY, folder)
    else localStorage.removeItem(STORAGE_KEY)
  } catch {
    // Storage can be unavailable; the choice still applies until the app closes.
  }
}

/** The folder used when none was chosen: `DBForge` in the user's Documents. */
export function defaultExportFolder(documents: string): string {
  return joinPath(documents, 'DBForge')
}

/** `name` inside `folder`, with the folder's own separator (`\` on Windows). */
export function joinPath(folder: string, name: string): string {
  const separator = folder.includes('\\') && !folder.includes('/') ? '\\' : '/'
  return folder.replace(/[\\/]+$/, '') + separator + name
}

/** A file name for a database's export, unique to the minute: `shop-2026-11-10-1432.sql`. */
export function exportFileName(database: string, at: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  const date = `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`
  // Characters a file name can't hold on any system become dashes.
  const safe = database.replace(/[\\/:*?"<>|]/g, '-')
  return `${safe}-${date}-${pad(at.getHours())}${pad(at.getMinutes())}.sql`
}
