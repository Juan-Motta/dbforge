import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '../components/Button'
import { Modal } from '../components/Modal'
import { useMessages } from '../lib/i18n/useLanguage'
import { messages } from './messages'
import styles from './SetAsideDialog.module.css'

/**
 * Tells the user, once at start, about settings files DBForge couldn't read:
 * each was moved aside and kept, and the app started without it.
 */
export function SetAsideDialog() {
  const m = useMessages(messages)
  const [files, setFiles] = useState<string[]>([])

  useEffect(() => {
    let current = true
    invoke<string[]>('set_aside_files')
      .then((paths) => current && setFiles(paths))
      .catch(() => {})
    return () => {
      current = false
    }
  }, [])

  if (files.length === 0) return null
  const close = () => setFiles([])
  return (
    <Modal title={m.setAsideTitle} onClose={close} className={styles.dialog}>
      <div className={styles.body}>
        <p>{m.setAsideNote}</p>
        <ul className={styles.files}>
          {files.map((file) => (
            <li key={file}>{file}</li>
          ))}
        </ul>
      </div>
      <footer className={styles.footer}>
        <Button variant="primary" data-autofocus onClick={close}>
          {m.setAsideOk}
        </Button>
      </footer>
    </Modal>
  )
}
