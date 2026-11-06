import type { ReactNode } from 'react'
import { Icon, type IconName } from './Icon'
import styles from './TabStrip.module.css'

export interface TabItem {
  id: string
  title: string
  icon: IconName
}

interface TabStripProps {
  tabs: TabItem[]
  activeId: string | null
  onSelect: (id: string) => void
  onClose: (id: string) => void
  /** Rendered after the tabs, e.g. a "new tab" button. */
  children?: ReactNode
}

export function TabStrip({ tabs, activeId, onSelect, onClose, children }: TabStripProps) {
  return (
    <div role="tablist" className={styles.strip}>
      {tabs.map((tab) => (
        <div
          key={tab.id}
          role="tab"
          tabIndex={0}
          aria-selected={tab.id === activeId}
          className={styles.tab}
          onClick={() => onSelect(tab.id)}
          // Middle click closes, as in browsers.
          onAuxClick={(event) => event.button === 1 && onClose(tab.id)}
        >
          <Icon name={tab.icon} size={15} className={styles.icon} />
          {tab.title}
          <button
            type="button"
            aria-label={`Close ${tab.title}`}
            className={styles.close}
            onClick={(event) => {
              event.stopPropagation()
              onClose(tab.id)
            }}
          >
            <Icon name="close" size={12} />
          </button>
        </div>
      ))}
      {children}
    </div>
  )
}
