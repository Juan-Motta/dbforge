import { useState } from 'react'
import { IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages, type McpMessages } from './messages'
import {
  AGENTS,
  agentPrompt,
  instructionsFile,
  projectNote,
  setupSteps,
  type AgentId,
  type Endpoint,
  type Step,
} from './setup'
import styles from './Mcp.module.css'

type Transport = Endpoint['transport']

interface AgentSetupProps {
  url: string
  /** The executable agents start over stdio; `null` leaves HTTP as the only way. */
  executable: string | null
  /** `null` until it is read; the blocks can't be copied meanwhile. */
  token: string | null
  /** Shows the token written in; otherwise `<token>` stands for it (copies always carry it). */
  showToken: boolean
  onCopy: (id: string, text: string) => void
  copied: string | null
}

const PLACEHOLDER = '<token>'

/**
 * Pick an agent and how it reaches DBForge, then: the steps that add DBForge
 * to it, a prompt that has the agent do it itself, and the note that has it
 * use DBForge in a project. Over stdio (the default) nothing secret is
 * written in; over HTTP the token is.
 */
export function AgentSetup({ url, executable, token, showToken, onCopy, copied }: AgentSetupProps) {
  const m = useMessages(messages)
  const [agent, setAgent] = useState<AgentId>('claude')
  const [chosen, setTransport] = useState<Transport>('stdio')
  const transport: Transport = executable === null ? 'http' : chosen
  const endpoint = (secret: string): Endpoint =>
    transport === 'stdio' && executable !== null
      ? { transport: 'stdio', executable }
      : { transport: 'http', url, token: secret }
  const shown = endpoint(showToken && token ? token : PLACEHOLDER)
  const copyEndpoint = endpoint(token ?? '')
  const steps = setupSteps(agent, shown)
  const copies = setupSteps(agent, copyEndpoint)
  // Over stdio nothing waits on the token, so the blocks copy at once.
  const ready = transport === 'stdio' || token !== null

  const block = (id: string, title: string, text: string, copy: string, tall = false) => (
    <div key={id} className={styles.command}>
      <span className={styles.commandTitle}>{title}</span>
      <div className={styles.row}>
        <code className={tall ? `${styles.value} ${styles.tall}` : styles.value}>{text}</code>
        <IconButton label={m.copy} disabled={!ready} onClick={() => onCopy(id, copy)}>
          <Icon name={copied === id ? 'check' : 'duplicate'} />
        </IconButton>
      </div>
    </div>
  )

  return (
    <section className={styles.section}>
      <h4 className={styles.sectionTitle} id="mcp-agent">
        {m.addToAgents}
      </h4>
      <div role="radiogroup" aria-labelledby="mcp-agent" className={styles.choices}>
        {AGENTS.map((id) => (
          <button
            key={id}
            type="button"
            role="radio"
            aria-checked={agent === id}
            onClick={() => setAgent(id)}
          >
            {m[AGENT_LABELS[id]]}
          </button>
        ))}
      </div>

      {executable !== null && (
        <>
          <span className={styles.commandTitle} id="mcp-transport">
            {m.transport}
          </span>
          <div role="radiogroup" aria-labelledby="mcp-transport" className={styles.choices}>
            {TRANSPORTS.map((id) => (
              <button
                key={id}
                type="button"
                role="radio"
                aria-checked={transport === id}
                onClick={() => setTransport(id)}
              >
                {id === 'stdio' ? m.transportStdio : m.transportHttp}
              </button>
            ))}
          </div>
          <p className={styles.note}>{transport === 'stdio' ? m.transportStdioNote : m.transportHttpNote}</p>
        </>
      )}

      {steps.map((step, index) =>
        block(
          `${agent}-step-${index}`,
          stepTitle(step, m),
          step.text,
          copies[index].text,
          step.text.includes('\n'),
        ),
      )}

      {block(
        `${agent}-prompt`,
        m.promptTitle,
        agentPrompt(agent, shown),
        agentPrompt(agent, copyEndpoint),
        true,
      )}
      <p className={styles.note}>{transport === 'stdio' ? m.promptHelpStdio : m.promptHelp}</p>

      {block(`${agent}-note`, m.noteTitle(instructionsFile(agent)), projectNote(), projectNote(), true)}
    </section>
  )
}

const TRANSPORTS: Transport[] = ['stdio', 'http']

const AGENT_LABELS: Record<AgentId, 'claudeCode' | 'codex' | 'opencode' | 'pi' | 'otherAgent'> = {
  claude: 'claudeCode',
  codex: 'codex',
  opencode: 'opencode',
  pi: 'pi',
  other: 'otherAgent',
}

function stepTitle(step: Step, m: McpMessages): string {
  switch (step.kind) {
    case 'terminal':
      return m.stepTerminal
    case 'profile':
      return m.stepProfile
    case 'config':
      return m.stepConfig(step.file ?? '')
    case 'connect':
      return m.stepConnect
    case 'launch':
      return m.stepLaunch
  }
}
