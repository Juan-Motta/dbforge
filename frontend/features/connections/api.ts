import { invoke } from '@tauri-apps/api/core'
import type { Connection } from '../../domain/connection'

/** Sent to the backend to create (no id), update or test a connection. */
export interface ConnectionInput extends Omit<Connection, 'id'> {
  id?: string
  /** Omit to keep the stored password. */
  password?: string
}

/** Where connection passwords are kept. */
export type CredentialStorage = 'keychain' | 'file'

export interface CredentialStorageInfo {
  storage: CredentialStorage
  /** Where the encrypted file is, or would be. */
  filePath: string
}

export const connectionsApi = {
  list: () => invoke<Connection[]>('list_connections'),
  save: (input: ConnectionInput) => invoke<Connection>('save_connection', { input }),
  remove: (id: string) => invoke<void>('delete_connection', { id }),
  /** Resolves with the server version, e.g. "PostgreSQL 16.2". */
  test: (input: ConnectionInput) => invoke<string>('test_connection', { input }),
  credentialStorage: () => invoke<CredentialStorageInfo>('credential_storage'),
  /** Moves every saved password to `storage`; nothing changes if one can't be moved. */
  setCredentialStorage: (storage: CredentialStorage) =>
    invoke<CredentialStorageInfo>('set_credential_storage', { storage }),
}
