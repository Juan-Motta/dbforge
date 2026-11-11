import { Select } from '../../components/Select'
import { encrypts, ENGINES, sslModeWorksThroughSsh, type Ssl, type SslMode } from '../../domain/connection'
import { FileField } from './FileField'
import { clientCertIncomplete, sslBlockedByTunnel, sslModesOf, tunnelOn, type ConnectionForm } from './form'
import type { ConnectionMessages } from './messages'
import styles from './ConnectionDialog.module.css'

interface SslSectionProps {
  form: ConnectionForm
  onChange: (ssl: Ssl) => void
  m: ConnectionMessages
}

/** The SSL mode, with what it checks, and the certificate files the engine can use. */
export function SslSection({ form, onChange, m }: SslSectionProps) {
  const { ssl } = form
  const support = ENGINES[form.engine].ssl
  const tunnel = tunnelOn(form)
  const filesOff = !encrypts(ssl.mode)
  const set = (changes: Partial<Ssl>) => onChange({ ...ssl, ...changes })
  const filters = [
    { name: m.pemFiles, extensions: ['pem', 'crt', 'cer', 'key'] },
    { name: m.allFiles, extensions: ['*'] },
  ]
  const blocked = sslBlockedByTunnel(form)

  return (
    <>
      <div>
        <Select
          label={m.sslMode}
          info={m.sslModeInfo}
          value={ssl.mode}
          options={sslModesOf(form).map((mode) => ({
            value: mode,
            label:
              modeLabel(mode, m) +
              (tunnel && !sslModeWorksThroughSsh(form.engine, mode) ? m.notThroughSsh : ''),
          }))}
          onChange={(mode) => set({ mode })}
        />
        {/* Two lines are kept for whichever text shows, so changing the mode never moves the fields. */}
        <p className={blocked ? `${styles.note} ${styles.warnNote}` : styles.note} role="status">
          {blocked ? m.verifyFullThroughSsh(ENGINES[form.engine].label) : modeHelp(ssl.mode, m)}
        </p>
      </div>

      {support.caFile && (
        <FileField
          label={m.caFile}
          info={m.caFileInfo}
          placeholder={m.noFile}
          browseLabel={m.browse}
          filters={filters}
          disabled={filesOff}
          value={ssl.caFile}
          onChange={(caFile) => set({ caFile })}
        />
      )}
      {support.clientCert &&
        (support.separateKey ? (
          <>
            <FileField
              label={m.clientCert}
              info={m.clientCertInfo}
              placeholder={m.noFile}
              browseLabel={m.browse}
              filters={filters}
              disabled={filesOff}
              invalid={clientCertIncomplete(form) && ssl.certFile.trim() === ''}
              value={ssl.certFile}
              onChange={(certFile) => set({ certFile })}
            />
            <FileField
              label={m.clientKey}
              info={m.clientKeyInfo}
              help={m.clientKeyHelp}
              placeholder={m.noFile}
              browseLabel={m.browse}
              filters={filters}
              disabled={filesOff}
              invalid={clientCertIncomplete(form) && ssl.keyFile.trim() === ''}
              value={ssl.keyFile}
              onChange={(keyFile) => set({ keyFile })}
            />
          </>
        ) : (
          <FileField
            label={m.mongoClientCert}
            info={m.mongoClientCertInfo}
            placeholder={m.noFile}
            browseLabel={m.browse}
            filters={filters}
            disabled={filesOff}
            value={ssl.certFile}
            onChange={(certFile) => set({ certFile })}
          />
        ))}
    </>
  )
}

function modeLabel(mode: SslMode, m: ConnectionMessages): string {
  const labels: Record<SslMode, string> = {
    disable: m.sslDisable,
    prefer: m.sslPrefer,
    require: m.sslRequire,
    'verify-ca': m.sslVerifyCa,
    'verify-full': m.sslVerifyFull,
  }
  return labels[mode]
}

function modeHelp(mode: SslMode, m: ConnectionMessages): string {
  const help: Record<SslMode, string> = {
    disable: m.sslDisableHelp,
    prefer: m.sslPreferHelp,
    require: m.sslRequireHelp,
    'verify-ca': m.sslVerifyCaHelp,
    'verify-full': m.sslVerifyFullHelp,
  }
  return help[mode]
}
