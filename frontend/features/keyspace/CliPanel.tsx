import { useEffect, useRef, useState } from 'react'
import { IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { queryApi } from '../query/api'
import { messages } from './messages'
import styles from './KeyBrowser.module.css'

interface Line {
  kind: 'command' | 'reply'
  text: string
}

interface CliPanelProps {
  sessionId: string
  connectionId: string
  db: number
  /** `host:port · db0`. */
  address: string
  /** A command ran: what the browser shows may have changed. */
  onRan: () => void
}

/** redis-cli in a panel: one command per Enter, replies printed as redis-cli prints them. */
export function CliPanel({ sessionId, connectionId, db, address, onRan }: CliPanelProps) {
  const m = useMessages(messages)
  const [lines, setLines] = useState<Line[]>([])
  const [input, setInput] = useState('')
  const [history, setHistory] = useState<string[]>([])
  const [recall, setRecall] = useState<number | null>(null)
  const [running, setRunning] = useState(false)
  const output = useRef<HTMLDivElement>(null)

  // The session keeps the connection between commands; it closes with the tab.
  useEffect(() => () => void queryApi.closeSession(sessionId).catch(() => {}), [sessionId])
  useEffect(() => {
    output.current?.scrollTo({ top: output.current.scrollHeight })
  }, [lines])

  const run = async (command: string) => {
    if (!command.trim()) return
    setLines((current) => [...current, { kind: 'command', text: command }])
    setHistory((current) => [...current.filter((c) => c !== command), command])
    setRecall(null)
    setInput('')
    setRunning(true)
    try {
      const outcome = await queryApi.run(sessionId, connectionId, { database: String(db) }, command)
      const replies = outcome.results.flatMap((result) => result.rows.map((row) => row[0] ?? ''))
      setLines((current) => [...current, ...replies.map((text) => ({ kind: 'reply' as const, text }))])
      onRan()
    } catch (error) {
      setLines((current) => [...current, { kind: 'reply', text: `(error) ${errorMessage(error)}` }])
    } finally {
      setRunning(false)
    }
  }

  const browse = (step: -1 | 1) => {
    if (history.length === 0) return
    const index = recall === null ? (step < 0 ? history.length - 1 : null) : recall + step
    if (index === null || index >= history.length) {
      setRecall(null)
      setInput('')
    } else {
      const clamped = Math.max(index, 0)
      setRecall(clamped)
      setInput(history[clamped])
    }
  }

  return (
    <section className={styles.cli} aria-label={m.redisCli}>
      <div className={styles.cliBar}>
        <span className={styles.label}>CLI</span>
        <span className={styles.muted}>{address}</span>
        <span className={styles.spacer} />
        {running && (
          <IconButton
            label={m.cancelBlocking}
            onClick={() => void queryApi.cancel(sessionId).catch(() => {})}
          >
            <Icon name="stop" />
          </IconButton>
        )}
        <IconButton label={m.clearCli} onClick={() => setLines([])}>
          <Icon name="close" />
        </IconButton>
      </div>
      <div className={styles.cliOutput} ref={output} role="log" aria-label={m.cliOutput}>
        {lines.map((line, index) =>
          line.kind === 'command' ? (
            <div key={index}>
              <span className={styles.prompt}>›</span> {line.text}
            </div>
          ) : (
            <div key={index} className={replyClass(line.text)}>
              {line.text}
            </div>
          ),
        )}
        <form
          className={styles.cliInput}
          onSubmit={(event) => {
            event.preventDefault()
            if (!running) void run(input)
          }}
        >
          <span className={styles.prompt}>›</span>
          <input
            value={input}
            aria-label={m.redisCommand}
            placeholder={running ? m.running : m.cliPlaceholder}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setInput(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'ArrowUp') {
                event.preventDefault()
                browse(-1)
              } else if (event.key === 'ArrowDown') {
                event.preventDefault()
                browse(1)
              }
            }}
          />
        </form>
      </div>
    </section>
  )
}

/** Colors a reply line by what it holds, as the design does. */
function replyClass(text: string): string {
  const body = text.replace(/^\s*\d+\) /, '')
  if (body.startsWith('(error)')) return styles.replyError
  if (body.startsWith('(integer)') || body.startsWith('(double)')) return styles.replyNumber
  if (body.startsWith('"')) return styles.replyString
  return ''
}
