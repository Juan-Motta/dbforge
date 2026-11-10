import { useEffect, useState } from 'react'
import { Button, IconButton } from '../../components/Button'
import { Callout, CalloutSpace } from '../../components/Callout'
import { Icon } from '../../components/Icon'
import { Option, OptionList } from '../../components/OptionList'
import { TextField } from '../../components/TextField'
import { copyText } from '../../lib/clipboard'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { mcpApi, type McpState } from './api'
import { messages } from './messages'
import { claudeCommand, codexCommand, codexToken, parsePort, serverUrl } from './setup'
import styles from './Mcp.module.css'

/**
 * Settings › Agents: the MCP server on or off, its port, the token agents
 * send, and the commands that add DBForge to Claude Code and Codex.
 */
export function McpSettings() {
  const m = useMessages(messages)
  const [state, setState] = useState<McpState | null>(null)
  const [port, setPort] = useState('')
  const [token, setToken] = useState<string | null>(null)
  const [shown, setShown] = useState(false)
  const [regenerating, setRegenerating] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    mcpApi
      .state()
      .then((loaded) => {
        if (!current) return
        setState(loaded)
        setPort(String(loaded.settings.port))
      })
      .catch((failure) => current && setError(errorMessage(failure)))
    mcpApi
      .token()
      .then((value) => current && setToken(value))
      .catch((failure) => current && setError(errorMessage(failure)))
    return () => {
      current = false
    }
  }, [])

  const save = async (settings: McpState['settings']) => {
    setError(null)
    try {
      setState(await mcpApi.setSettings(settings))
    } catch (failure) {
      setError(errorMessage(failure))
    }
  }

  const applyPort = () => {
    const parsed = parsePort(port)
    if (!state || parsed === null || parsed === state.settings.port) return
    void save({ ...state.settings, port: parsed })
  }

  const copy = (id: string, text: string) =>
    void copyText(text).then(
      () => setCopied(id),
      (failure) => setError(errorMessage(failure)),
    )

  const regenerate = async () => {
    setRegenerating(false)
    try {
      setToken(await mcpApi.regenerateToken())
      setShown(true)
      // The server keeps running with the new token; read its state again rather than assume it.
      setState(await mcpApi.state())
    } catch (failure) {
      setError(errorMessage(failure))
    }
  }

  const url = serverUrl(state?.settings.port ?? 7419)
  const secret = token ?? ''
  const commands = [
    {
      id: 'claude',
      title: m.claudeCode,
      text: claudeCommand(url, shown ? secret : '<token>'),
      copy: claudeCommand(url, secret),
    },
    { id: 'codex', title: m.codex, text: codexCommand(url), copy: codexCommand(url) },
    {
      id: 'codexToken',
      title: m.codexToken,
      text: codexToken(shown ? secret : '<token>'),
      copy: codexToken(secret),
    },
  ]

  return (
    <div className={styles.settings}>
      <OptionList>
        <Option
          type="checkbox"
          icon="code"
          title={m.enable}
          description={m.enableDescription}
          checked={state?.settings.enabled ?? false}
          disabled={!state}
          onChange={(event) => state && void save({ ...state.settings, enabled: event.target.checked })}
        />
      </OptionList>

      {error ? (
        <Callout tone="failed" title={m.failed} detail={error} />
      ) : state?.status.state === 'running' ? (
        <Callout tone="done" title={m.listening} detail={state.status.url} mono />
      ) : state?.status.state === 'failed' ? (
        <Callout tone="failed" title={m.failed} detail={state.status.error} />
      ) : (
        <CalloutSpace />
      )}

      <TextField
        label={m.port}
        mono
        inputMode="numeric"
        value={port}
        help={parsePort(port) === null ? m.portInvalid : m.portHelp}
        aria-invalid={parsePort(port) === null}
        onChange={(event) => setPort(event.target.value)}
        onBlur={applyPort}
        onKeyDown={(event) => event.key === 'Enter' && applyPort()}
      />

      <section className={styles.section}>
        <h4 className={styles.sectionTitle}>{m.token}</h4>
        <div className={`${styles.row} ${regenerating ? styles.confirming : ''}`}>
          <Icon name={regenerating ? 'warning' : 'key'} size={14} />
          <span className={styles.value}>
            {regenerating ? m.regenerateConfirm : shown ? secret : '•'.repeat(32)}
          </span>
          {regenerating ? (
            <>
              <Button variant="ghost" onClick={() => setRegenerating(false)}>
                {m.cancel}
              </Button>
              <Button variant="primary" onClick={() => void regenerate()}>
                {m.regenerate}
              </Button>
            </>
          ) : (
            <>
              <IconButton label={shown ? m.hide : m.show} disabled={!token} onClick={() => setShown(!shown)}>
                <Icon name="eye" />
              </IconButton>
              <IconButton label={m.copy} disabled={!token} onClick={() => copy('token', secret)}>
                <Icon name={copied === 'token' ? 'check' : 'duplicate'} />
              </IconButton>
              <Button variant="ghost" disabled={!token} onClick={() => setRegenerating(true)}>
                <Icon name="refresh" />
                {m.regenerate}
              </Button>
            </>
          )}
        </div>
      </section>

      <section className={styles.section}>
        <h4 className={styles.sectionTitle}>{m.addToAgents}</h4>
        {commands.map((command) => (
          <div key={command.id} className={styles.command}>
            <span className={styles.commandTitle}>{command.title}</span>
            <div className={styles.row}>
              <code className={styles.value}>{command.text}</code>
              <IconButton label={m.copy} disabled={!token} onClick={() => copy(command.id, command.copy)}>
                <Icon name={copied === command.id ? 'check' : 'duplicate'} />
              </IconButton>
            </div>
          </div>
        ))}
        <p className={styles.note}>{m.accessNote}</p>
      </section>
    </div>
  )
}
