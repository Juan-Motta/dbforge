/** Whether the app runs on macOS, for names the system gives things (Finder, ⌘). */
export function isMac(): boolean {
  return /Mac/.test(navigator.userAgent)
}
