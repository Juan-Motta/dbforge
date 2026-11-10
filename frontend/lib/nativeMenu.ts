/** Where the system's own menu (cut, copy, paste) still helps: text fields and code editors. */
const EDITABLE = 'input, textarea, [contenteditable]:not([contenteditable="false"]), .cm-editor'

/** Whether a right-click on `target` keeps the system's menu. */
export function keepsNativeMenu(target: EventTarget | null): boolean {
  // Text nodes and the window have no `closest`; only elements can be editable.
  const element = target as Partial<Pick<Element, 'closest'>> | null
  return typeof element?.closest === 'function' && element.closest(EDITABLE) !== null
}

/**
 * Turns off the web view's own right-click menu ("Reload", "Inspect Element")
 * outside text fields. DBForge's menus open on their own right-clicks.
 */
export function suppressNativeMenu(target: Pick<Document, 'addEventListener'> = document) {
  target.addEventListener('contextmenu', (event) => {
    if (!keepsNativeMenu(event.target)) event.preventDefault()
  })
}
