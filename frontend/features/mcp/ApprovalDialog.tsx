import { Badge } from '../../components/Badge'
import { Button } from '../../components/Button'
import { SqlCode } from '../../components/code/SqlCode'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { sqlDialectOf, type Engine } from '../../domain/connection'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { ApprovalRequest } from './api'
import { messages } from './messages'
import styles from './Mcp.module.css'

interface ApprovalDialogProps {
  request: ApprovalRequest
  engine: Engine | undefined
  onAnswer: (approved: boolean) => void
}

/**
 * An agent wants to change data on a connection whose tag asks first: what it
 * will run, where, and who asks. Closing the dialog declines.
 */
export function ApprovalDialog({ request, engine, onAnswer }: ApprovalDialogProps) {
  const m = useMessages(messages)
  return (
    <Modal title={m.approvalTitle} onClose={() => onAnswer(false)} className={styles.approval}>
      <div className={styles.approvalBody}>
        <p className={styles.who}>
          <Icon name="warning" size={15} />
          <span>{m.approvalWho(request.agent ?? m.anAgent, request.connectionName)}</span>
          {request.tag && <Badge tone="err">{request.tag}</Badge>}
        </p>
        {request.database && <p className={styles.where}>{m.inDatabase(request.database)}</p>}
        <div className={styles.statement}>
          <SqlCode value={request.statement} dialect={sqlDialectOf(engine)} />
        </div>
        <p className={styles.note}>{m.approvalNote}</p>
      </div>
      <footer className={styles.footer}>
        <Button onClick={() => onAnswer(false)}>{m.decline}</Button>
        <Button variant="primary" data-autofocus onClick={() => onAnswer(true)}>
          {m.approve}
        </Button>
      </footer>
    </Modal>
  )
}
