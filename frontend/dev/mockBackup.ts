import type { BackupPreview, Exported, Imported } from '../features/backup/api'

/** The backup commands as the mock backend answers them: nothing is written or read. */
export function mockBackup(cmd: string): { handled: boolean; value?: unknown } {
  switch (cmd) {
    case 'export_backup':
      return { handled: true, value: { connections: 5, passwords: 4 } satisfies Exported }
    case 'read_backup':
      return {
        handled: true,
        value: {
          exportedAt: Date.now() - 86_400_000,
          appVersion: '0.1.0',
          preferences: { theme: 'light', language: 'es', exportFolder: '/Users/demo/Backups' },
          newConnections: ['analytics-pg', 'billing-mysql'],
          updatedConnections: ['demo-postgres'],
          passwords: 2,
          tags: 4,
          mcp: { enabled: true, port: 7419 },
          mcpToken: true,
          toolPaths: 1,
        } satisfies BackupPreview,
      }
    case 'import_backup':
      return {
        handled: true,
        value: {
          added: 2,
          updated: 1,
          withoutPassword: ['billing-mysql'],
          missingTools: [],
        } satisfies Imported,
      }
    default:
      return { handled: false }
  }
}
