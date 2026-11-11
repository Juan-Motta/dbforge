import { isTauri } from '@tauri-apps/api/core'

/**
 * On macOS the Tauri window uses an overlay title bar, so the native traffic
 * lights sit on top of our header and it needs room on the left. They are
 * centred in its 44px height by `trafficLightPosition` in tauri.conf.json,
 * whose `y` isn't their distance from the top: macOS keeps their own offset
 * within the title bar, so 24 was measured to centre them.
 */
export const hasOverlayTitleBar = isTauri() && navigator.userAgent.includes('Mac')
