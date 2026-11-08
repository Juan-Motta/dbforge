import { invoke } from '@tauri-apps/api/core'

/**
 * Whether key presses may hide the pointer. The macOS web view hides it on every
 * key the page handles, which suits typing but not moving around a grid.
 */
export function keepCursor(keep: boolean): void {
  void invoke('keep_cursor', { keep }).catch(() => {})
}
