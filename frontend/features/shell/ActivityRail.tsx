import { IconButton } from '../../components/Button'
import { Icon, type IconName } from '../../components/Icon'
import styles from './Shell.module.css'

export type Activity = 'explorer' | 'search' | 'history' | 'snippets' | 'diagram'

const ITEMS: { id: Activity; label: string; icon: IconName }[] = [
  { id: 'explorer', label: 'Explorer', icon: 'database' },
  { id: 'search', label: 'Search', icon: 'search' },
  { id: 'history', label: 'Query history', icon: 'history' },
  { id: 'snippets', label: 'Snippets', icon: 'code' },
  { id: 'diagram', label: 'ER diagram', icon: 'diagram' },
]

interface ActivityRailProps {
  active: Activity
  onChange: (activity: Activity) => void
}

export function ActivityRail({ active, onChange }: ActivityRailProps) {
  return (
    <nav className={`island ${styles.rail}`} aria-label="Activity">
      {ITEMS.map((item) => (
        <IconButton
          key={item.id}
          label={item.label}
          active={item.id === active}
          onClick={() => onChange(item.id)}
        >
          <Icon name={item.icon} />
        </IconButton>
      ))}
    </nav>
  )
}
