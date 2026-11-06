import { isTauri } from '@tauri-apps/api/core'

/**
 * On macOS the Tauri window uses an overlay title bar, so the native traffic
 * lights sit on top of our header and it needs room on the left.
 */
export const hasOverlayTitleBar = isTauri() && navigator.userAgent.includes('Mac')
