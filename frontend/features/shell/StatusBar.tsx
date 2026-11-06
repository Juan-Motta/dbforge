import { connectionAddress, ENGINE_LABELS, type Connection } from '../../domain/connection'
import styles from './Shell.module.css'

interface StatusBarProps {
  connection?: Connection
  path: string[]
}

export function StatusBar({ connection, path }: StatusBarProps) {
  return (
    <footer className={`island ${styles.statusBar}`}>
      {connection ? (
        <span className={styles.status}>
          {ENGINE_LABELS[connection.engine]} · {connectionAddress(connection)}
        </span>
      ) : (
        <span>No connection selected</span>
      )}
      {path.length > 0 && <span>{path.slice(0, 2).join('.')}</span>}
      <span className={styles.spacer} />
      <span>UTF-8</span>
    </footer>
  )
}
