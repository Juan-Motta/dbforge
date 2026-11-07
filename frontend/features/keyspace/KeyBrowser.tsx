import { useCallback, useEffect, useState } from 'react'
import { IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { errorMessage } from '../../lib/errors'
import { keyspaceApi, type Overview } from './api'
import { CliPanel } from './CliPanel'
import { FormDialog } from './FormDialog'
import { KeyDetail } from './KeyDetail'
import { formatBytes, hitRate, parseScore } from './keyspace'
import { KeyList } from './KeyList'
import styles from './KeyBrowser.module.css'

interface KeyBrowserProps {
  /** The tab's id; the CLI session is named after it. */
  tabId: string
  connectionId: string
  /** The database name the explorer shows: `db0`. */
  database: string
  /** `host:port`, for the CLI header. */
  address: string
}

/** A Redis database: its keys, the selected key, server metrics and a CLI. */
export function KeyBrowser({ tabId, connectionId, database, address }: KeyBrowserProps) {
  const db = Number(database.replace(/^db/, '')) || 0
  const [selected, setSelected] = useState<string | null>(null)
  const [overview, setOverview] = useState<Overview | null>(null)
  const [overviewError, setOverviewError] = useState<string | null>(null)
  const [keysToken, setKeysToken] = useState(0)
  const [valueToken, setValueToken] = useState(0)
  const [adding, setAdding] = useState(false)

  const refreshOverview = useCallback(async () => {
    try {
      setOverview(await keyspaceApi.overview(connectionId, db))
      setOverviewError(null)
    } catch (error) {
      setOverviewError(errorMessage(error))
    }
  }, [connectionId, db])

  useEffect(() => {
    // Ignores a reply that arrives after the tab moved to another database.
    let current = true
    keyspaceApi.overview(connectionId, db).then(
      (next) => current && (setOverview(next), setOverviewError(null)),
      (error) => current && setOverviewError(errorMessage(error)),
    )
    return () => {
      current = false
    }
  }, [connectionId, db])

  const keysChanged = (next: string | null) => {
    setSelected(next)
    setKeysToken((n) => n + 1)
    void refreshOverview()
  }

  return (
    <div className={styles.browser}>
      <KeyList
        connectionId={connectionId}
        db={db}
        selected={selected}
        onSelect={(key) => setSelected(key.name)}
        onAddKey={() => setAdding(true)}
        reloadToken={keysToken}
      />

      <div className={styles.side}>
        <div className={styles.metrics} aria-label="Server metrics">
          {overview ? (
            <>
              <Metric label="Keys" value={overview.keys.toLocaleString('en-US')} />
              <Metric
                label="Memory"
                value={formatBytes(overview.usedMemory)}
                detail={overview.maxMemory > 0 ? `/ ${formatBytes(overview.maxMemory)}` : undefined}
              />
              <Metric label="Ops / sec" value={overview.opsPerSec.toLocaleString('en-US')} />
              <Metric label="Hit rate" value={hitRate(overview.keyspaceHits, overview.keyspaceMisses)} ok />
              <Metric label="Clients" value={overview.clients.toLocaleString('en-US')} />
            </>
          ) : (
            <p className={overviewError ? styles.error : styles.muted}>
              {overviewError ?? 'Reading server metrics…'}
            </p>
          )}
          <IconButton
            label="Refresh metrics"
            className={styles.refresh}
            onClick={() => void refreshOverview()}
          >
            <Icon name="refresh" />
          </IconButton>
        </div>

        <main className={styles.main}>
          {selected ? (
            <KeyDetail
              // A new key starts with its first element picked.
              key={selected}
              connectionId={connectionId}
              db={db}
              name={selected}
              reloadToken={valueToken}
              onKeysChanged={keysChanged}
            />
          ) : (
            <p className={styles.detailEmpty}>Select a key to see its value.</p>
          )}
        </main>

        <CliPanel
          sessionId={`${tabId}:cli`}
          connectionId={connectionId}
          db={db}
          address={`${address} · ${database}`}
          onRan={() => {
            setValueToken((n) => n + 1)
            void refreshOverview()
          }}
        />

        {overview && (
          <footer className={styles.footer}>
            <span className={styles.dot} />
            Connected · Redis {overview.version} · {overview.mode}
            <span className={styles.spacer} />
            Evictions {overview.evictedKeys.toLocaleString('en-US')} · Expired{' '}
            {overview.expiredKeys.toLocaleString('en-US')}
          </footer>
        )}
      </div>

      {adding && (
        <FormDialog
          title="Add key"
          submitLabel="Add key"
          fields={[
            { name: 'name', label: 'Key name', placeholder: 'session:7f3a9c21e8' },
            {
              name: 'kind',
              label: 'Type',
              type: 'select',
              initial: 'string',
              options: ['string', 'hash', 'list', 'set', 'zset', 'stream'].map((k) => ({
                value: k,
                label: k,
              })),
            },
            { name: 'field', label: 'First field', when: (v) => v.kind === 'hash' || v.kind === 'stream' },
            { name: 'value', label: 'Value', type: 'textarea' },
            { name: 'score', label: 'Score', type: 'number', when: (v) => v.kind === 'zset' },
            { name: 'ttl', label: 'Expires in (seconds, optional)', type: 'number' },
          ]}
          onSubmit={async (form) => {
            const ttl = form.ttl.trim() === '' ? null : Number(form.ttl)
            if (ttl !== null && (!Number.isFinite(ttl) || ttl <= 0))
              throw 'Enter a number of seconds greater than 0.'
            await keyspaceApi.createKey(connectionId, db, {
              name: form.name,
              kind: form.kind,
              field: form.kind === 'hash' || form.kind === 'stream' ? form.field : null,
              value: form.value,
              score: form.kind === 'zset' ? parseScore(form.score) : null,
              ttlMs: ttl === null ? null : Math.round(ttl * 1000),
            })
            keysChanged(form.name)
          }}
          onClose={() => setAdding(false)}
        />
      )}
    </div>
  )
}

function Metric({
  label,
  value,
  detail,
  ok,
}: {
  label: string
  value: string
  detail?: string
  ok?: boolean
}) {
  return (
    <div className={styles.metric}>
      <div className={styles.label}>{label}</div>
      <div className={`${styles.metricValue} ${ok ? styles.ok : ''}`}>
        {value} {detail && <span className={styles.metricDetail}>{detail}</span>}
      </div>
    </div>
  )
}
