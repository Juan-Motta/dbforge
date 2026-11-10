import { useCallback, useEffect, useState } from 'react'
import { IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { keyspaceApi, type Overview } from './api'
import { CliPanel } from './CliPanel'
import { FormDialog } from './FormDialog'
import { KeyDetail } from './KeyDetail'
import { formatBytes, hitRate, parseScore } from './keyspace'
import { KeyList } from './KeyList'
import { messages } from './messages'
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
  const m = useMessages(messages)
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
        <div className={styles.metrics} aria-label={m.serverMetrics}>
          {overview ? (
            <>
              <Metric label={m.keys} value={overview.keys.toLocaleString(m.locale)} />
              <Metric
                label={m.memory}
                value={formatBytes(overview.usedMemory)}
                detail={overview.maxMemory > 0 ? `/ ${formatBytes(overview.maxMemory)}` : undefined}
              />
              <Metric label={m.opsPerSec} value={overview.opsPerSec.toLocaleString(m.locale)} />
              <Metric label={m.hitRate} value={hitRate(overview.keyspaceHits, overview.keyspaceMisses)} ok />
              <Metric label={m.clients} value={overview.clients.toLocaleString(m.locale)} />
            </>
          ) : (
            <p className={overviewError ? styles.error : styles.muted}>{overviewError ?? m.readingMetrics}</p>
          )}
          <IconButton
            label={m.refreshMetrics}
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
            <p className={styles.detailEmpty}>{m.selectKey}</p>
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
            {m.connectedTo(overview.version, overview.mode)}
            <span className={styles.spacer} />
            {m.evictions} {overview.evictedKeys.toLocaleString(m.locale)} · {m.expired}{' '}
            {overview.expiredKeys.toLocaleString(m.locale)}
          </footer>
        )}
      </div>

      {adding && (
        <FormDialog
          title={m.addKey}
          submitLabel={m.addKey}
          fields={[
            { name: 'name', label: m.keyName, placeholder: 'session:7f3a9c21e8' },
            {
              name: 'kind',
              label: m.type,
              type: 'select',
              initial: 'string',
              options: ['string', 'hash', 'list', 'set', 'zset', 'stream'].map((k) => ({
                value: k,
                label: k,
              })),
            },
            { name: 'field', label: m.firstField, when: (v) => v.kind === 'hash' || v.kind === 'stream' },
            { name: 'value', label: m.value, type: 'textarea' },
            { name: 'score', label: m.score, type: 'number', when: (v) => v.kind === 'zset' },
            { name: 'ttl', label: m.expiresInOptional, type: 'number' },
          ]}
          onSubmit={async (form) => {
            const ttl = form.ttl.trim() === '' ? null : Number(form.ttl)
            if (ttl !== null && (!Number.isFinite(ttl) || ttl <= 0)) throw m.secondsGreaterThanZero
            await keyspaceApi.createKey(connectionId, db, {
              name: form.name,
              kind: form.kind,
              field: form.kind === 'hash' || form.kind === 'stream' ? form.field : null,
              value: form.value,
              score: form.kind === 'zset' ? parseScore(form.score, m) : null,
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
