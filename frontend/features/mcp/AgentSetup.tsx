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
  type Step,
} from './setup'
import styles from './Mcp.module.css'

interface AgentSetupProps {
  url: string
  /** `null` until it is read; the blocks can't be copied meanwhile. */
  token: string | null
  /** Shows the token written in; otherwise `<token>` stands for it (copies always carry it). */
  showToken: boolean
  onCopy: (id: string, text: string) => void
  copied: string | null
}

const PLACEHOLDER = '<token>'

/**
 * Pick an agent, then: the steps that add DBForge to it, a prompt that has
 * the agent do it itself, and the note that has it use DBForge in a project.
 */
export function AgentSetup({ url, token, showToken, onCopy, copied }: AgentSetupProps) {
  const m = useMessages(messages)
  const [agent, setAgent] = useState<AgentId>('claude')
  const shown = showToken && token ? token : PLACEHOLDER
  const steps = setupSteps(agent, url, shown)
  const copies = setupSteps(agent, url, token ?? '')

  const block = (id: string, title: string, text: string, copy: string, tall = false) => (
    <div key={id} className={styles.command}>
      <span className={styles.commandTitle}>{title}</span>
      <div className={styles.row}>
        <code className={tall ? `${styles.value} ${styles.tall}` : styles.value}>{text}</code>
        <IconButton label={m.copy} disabled={!token} onClick={() => onCopy(id, copy)}>
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
      <div role="radiogroup" aria-labelledby="mcp-agent" className={styles.agents}>
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
        agentPrompt(agent, url, shown),
        agentPrompt(agent, url, token ?? ''),
        true,
      )}
      <p className={styles.note}>{m.promptHelp}</p>

      {block(`${agent}-note`, m.noteTitle(instructionsFile(agent)), projectNote(), projectNote(), true)}
    </section>
  )
}

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
  }
}
