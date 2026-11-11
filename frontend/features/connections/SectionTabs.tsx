import type { KeyboardEvent } from 'react'
import { tabId, panelId } from './sectionIds'
import styles from './ConnectionDialog.module.css'

export interface SectionTab<T extends string> {
  id: T
  label: string
  /** Its setting is on: a dot shows it, and `title` says what. */
  active?: boolean
  title?: string
}

interface SectionTabsProps<T extends string> {
  label: string
  /** The prefix of each tab's and panel's id, as `panelId` builds them. */
  idPrefix: string
  tabs: SectionTab<T>[]
  selected: T
  onSelect: (id: T) => void
}

/**
 * The form's sections, as tabs: one tab stop, with the arrow keys, Home and End
 * moving between them as WAI-ARIA tabs do.
 */
export function SectionTabs<T extends string>({
  label,
  idPrefix,
  tabs,
  selected,
  onSelect,
}: SectionTabsProps<T>) {
  const onKeyDown = (event: KeyboardEvent) => {
    const index = tabs.findIndex((tab) => tab.id === selected)
    const next: Record<string, number> = {
      ArrowRight: (index + 1) % tabs.length,
      ArrowLeft: (index - 1 + tabs.length) % tabs.length,
      Home: 0,
      End: tabs.length - 1,
    }
    if (!(event.key in next)) return
    event.preventDefault()
    const tab = tabs[next[event.key]]
    onSelect(tab.id)
    document.getElementById(tabId(idPrefix, tab.id))?.focus()
  }

  return (
    <div role="tablist" aria-label={label} className={styles.tabs} onKeyDown={onKeyDown}>
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          id={tabId(idPrefix, tab.id)}
          aria-selected={tab.id === selected}
          aria-controls={panelId(idPrefix, tab.id)}
          tabIndex={tab.id === selected ? 0 : -1}
          title={tab.active ? tab.title : undefined}
          className={styles.tab}
          onClick={() => onSelect(tab.id)}
        >
          {tab.label}
          {/* Always there, so turning a setting on never widens the tab. */}
          <span className={styles.tabDot} data-on={tab.active ? '' : undefined} aria-hidden />
        </button>
      ))}
    </div>
  )
}
