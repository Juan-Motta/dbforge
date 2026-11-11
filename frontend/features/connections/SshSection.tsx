import { Select } from '../../components/Select'
import { TextField } from '../../components/TextField'
import type { SshAuth } from '../../domain/connection'
import { CheckOption } from './CheckOption'
import { FileField } from './FileField'
import { isValidPort, tunnelUnreachable, type ConnectionForm, type SshForm } from './form'
import type { ConnectionMessages } from './messages'
import styles from './ConnectionDialog.module.css'

interface SshSectionProps {
  form: ConnectionForm
  onChange: (changes: { ssh?: SshForm; sshSecret?: string }) => void
  /** Editing a saved connection: an empty secret keeps the saved one. */
  editing: boolean
  m: ConnectionMessages
}

/**
 * The SSH tunnel: on or off, the server, its user and how it signs in. Every
 * field stays in place whatever the method, so switching never moves the form.
 */
export function SshSection({ form, onChange, editing, m }: SshSectionProps) {
  const { ssh } = form
  const set = (changes: Partial<SshForm>) => onChange({ ssh: { ...ssh, ...changes } })
  const unreachable = tunnelUnreachable(form)

  return (
    <>
      <CheckOption
        label={m.useSsh}
        help={
          <span className={unreachable ? styles.warnText : undefined}>
            {unreachable ? m.sshNeedsHost : m.useSshHelp}
          </span>
        }
        checked={ssh.enabled}
        onChange={(enabled) => set({ enabled })}
      />
      {/* A disabled fieldset disables every field and button in it while the tunnel is off. */}
      <fieldset className={styles.section} disabled={!ssh.enabled}>
        <div className={styles.hostRow}>
          <TextField
            label={m.sshHost}
            info={m.sshHostInfo}
            mono
            placeholder="bastion.example.com"
            value={ssh.host}
            onChange={(e) => set({ host: e.target.value })}
          />
          <TextField
            label={m.port}
            info={m.sshPortInfo}
            mono
            inputMode="numeric"
            aria-invalid={ssh.enabled && !isValidPort(ssh.port)}
            value={ssh.port}
            onChange={(e) => set({ port: e.target.value })}
          />
        </div>
        <div className={styles.pair}>
          <TextField
            label={m.sshUser}
            mono
            autoComplete="off"
            value={ssh.username}
            onChange={(e) => set({ username: e.target.value })}
          />
          <Select<SshAuth>
            label={m.sshAuth}
            info={m.sshAuthInfo}
            value={ssh.auth}
            options={[
              { value: 'password', label: m.sshAuthPassword },
              { value: 'key', label: m.sshAuthKey },
              { value: 'agent', label: m.sshAuthAgent },
            ]}
            onChange={(auth) => set({ auth })}
          />
        </div>
        <FileField
          label={m.sshKeyFile}
          info={m.sshKeyFileInfo}
          placeholder={m.sshKeyFilePlaceholder}
          browseLabel={m.browse}
          filters={[{ name: m.sshKeyFiles, extensions: ['*'] }]}
          disabled={!ssh.enabled || ssh.auth !== 'key'}
          invalid={ssh.enabled && ssh.auth === 'key' && ssh.keyFile.trim() === ''}
          value={ssh.keyFile}
          onChange={(keyFile) => set({ keyFile })}
        />
        <TextField
          label={ssh.auth === 'key' ? m.sshPassphrase : m.sshPassword}
          info={m.sshSecretInfo}
          mono
          type="password"
          autoComplete="new-password"
          disabled={ssh.auth === 'agent'}
          placeholder={ssh.auth === 'agent' ? m.sshNoSecret : editing ? m.unchanged : ''}
          value={ssh.auth === 'agent' ? '' : form.sshSecret}
          onChange={(e) => onChange({ sshSecret: e.target.value })}
        />
      </fieldset>
      <p className={styles.note}>{m.hostKeysHelp}</p>
    </>
  )
}
