import { useEffect, useRef, useState } from 'react'
import { Badge } from '../../components/Badge'
import { Button } from '../../components/Button'
import { EngineLogo } from '../../components/EngineLogo'
import { Icon } from '../../components/Icon'
import type { Connection } from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import { connectionsApi } from '../connections/api'
import { connectionFacts } from './connectionFacts'
import styles from './Shell.module.css'

interface ConnectionDetailsProps {
  connection: Connection
  onEdit: () => void
}

/** The title bar's connection chip, which opens what the status bar used to show. */
export function ConnectionDetails({ connection, onEdit }: ConnectionDetailsProps) {
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)

  // Closes on a click elsewhere or Escape, as menus do.
  useEffect(() => {
    if (!open) return
    const onPointer = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false)
    }
    const onKey = (event: KeyboardEvent) => event.key === 'Escape' && setOpen(false)
    document.addEventListener('pointerdown', onPointer)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('pointerdown', onPointer)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  return (
    <div className={styles.chip} ref={root}>
      <Button className={styles.connection} aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <EngineLogo engine={connection.engine} />
        {connection.name}
        <EnvironmentBadge connection={connection} />
        <Icon name="chevronDown" size={12} className="muted" />
      </Button>
      {open && (
        <Popover
          key={connection.id}
          connection={connection}
          onEdit={() => {
            setOpen(false)
            onEdit()
          }}
        />
      )}
    </div>
  )
}

function Popover({ connection, onEdit }: ConnectionDetailsProps) {
  const [server, setServer] = useState<{ version: string } | { error: string } | null>(null)

  // Asks the server each time the popover opens: it also tells whether it is reachable.
  useEffect(() => {
    let current = true
    connectionsApi.test({ ...connection, password: undefined }).then(
      (version) => current && setServer({ version }),
      (error) => current && setServer({ error: errorMessage(error) }),
    )
    return () => {
      current = false
    }
  }, [connection])

  const reachable = server && 'version' in server
  return (
    <div role="dialog" aria-label={`${connection.name} connection`} className={styles.popover}>
      <div className={styles.popoverHead}>
        <EngineLogo engine={connection.engine} size={22} />
        <div className={styles.popoverTitle}>
          <strong>{connection.name}</strong>
          <span className={styles.state}>
            <span
              className={styles.dot}
              style={{ background: reachable ? 'var(--ok)' : server ? 'var(--err)' : 'var(--text3)' }}
            />
            {reachable ? 'Reachable' : server ? 'Not reachable' : 'Checking…'}
          </span>
        </div>
        <EnvironmentBadge connection={connection} />
      </div>
      <dl className={styles.facts}>
        {connectionFacts(
          connection,
          server && 'version' in server ? server.version : server ? '—' : null,
        ).map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      {server && 'error' in server && <p className={styles.popoverError}>{server.error}</p>}
      <div className={styles.popoverActions}>
        <Button onClick={onEdit}>
          <Icon name="edit" />
          Edit connection
        </Button>
      </div>
    </div>
  )
}

function EnvironmentBadge({ connection }: { connection: Connection }) {
  if (connection.environment === 'production') return <Badge tone="err">PROD</Badge>
  if (connection.environment === 'staging') return <Badge tone="warn">STAGE</Badge>
  if (connection.environment === 'development') return <Badge tone="info">DEV</Badge>
  return null
}
