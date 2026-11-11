/** The id of a section's tab, under the dialog's own `prefix`. */
export const tabId = (prefix: string, id: string) => `${prefix}-tab-${id}`

/** The id of the panel a section's tab shows. */
export const panelId = (prefix: string, id: string) => `${prefix}-panel-${id}`
