import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Connection } from '../../domain/connection'

/** Sent to the backend to create (no id), update or test a connection. */
export interface ConnectionInput extends Omit<Connection, 'id'> {
  id?: string
  /** Omit to keep the stored password; empty clears it. */
  password?: string
  /** The SSH password, or the private key's passphrase; omitted or empty as `password`. */
  sshSecret?: string
}

/** Where connection passwords are kept. */
export type CredentialStorage = 'keychain' | 'file'

export interface CredentialStorageInfo {
  storage: CredentialStorage
  /** Where the encrypted file is, or would be. */
  filePath: string
}

/** Where a storage switch is: the password it is handling now, of `total`. */
export interface SwitchProgress {
  /** Copying into the new place, or removing the copies left in the old one. */
  phase: 'moving' | 'removing'
  current: number
  total: number
}

export const connectionsApi = {
  list: () => invoke<Connection[]>('list_connections'),
  save: (input: ConnectionInput) => invoke<Connection>('save_connection', { input }),
  remove: (id: string) => invoke<void>('delete_connection', { id }),
  /** A copy named `name`, password included. */
  duplicate: (id: string, name: string) => invoke<Connection>('duplicate_connection', { id, name }),
  /** Resolves with the server version, e.g. "PostgreSQL 16.2". */
  test: (input: ConnectionInput) => invoke<string>('test_connection', { input }),
  credentialStorage: () => invoke<CredentialStorageInfo>('credential_storage'),
  /** Moves every saved password to `storage`; nothing changes if one can't be moved. */
  setCredentialStorage: (storage: CredentialStorage) =>
    invoke<CredentialStorageInfo>('set_credential_storage', { storage }),
  /** Each step of a storage switch; resolves to the function that stops listening. */
  onSwitchProgress: (handler: (progress: SwitchProgress) => void) =>
    listen<SwitchProgress>('credential-progress', (event) => handler(event.payload)),
}
