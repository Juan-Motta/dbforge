import { defineMessages } from '../../lib/i18n/messages'

export const messages = defineMessages({
  en: {
    exportTitle: 'Export',
    includeSecrets: 'Include passwords and the MCP token',
    includeSecretsDescription:
      'Written in plain text: anyone with the file can read them. Keep it somewhere safe.',
    exportButton: 'Export configuration…',
    exported: (connections: number, passwords: number) =>
      `Exported ${connections} ${connections === 1 ? 'connection' : 'connections'}, ${passwords} with ${passwords === 1 ? 'its password' : 'their passwords'}`,
    importTitle: 'Import',
    importHelp:
      'Adds what the file holds to this installation: connections and tags already here are updated, nothing is deleted.',
    chooseFile: 'Choose a backup…',
    fromFile: (path: string, when: string, version: string) =>
      `${path} · exported ${when} by DBForge ${version}`,
    partPreferences: 'Preferences',
    partPreferencesDetail: (theme: string, language: string, folder: string | undefined) =>
      `Theme ${theme}, language ${language}${folder ? `, exports to ${folder}` : ''}`,
    partConnections: (added: number, updated: number) => `Connections: ${added} new, ${updated} updated`,
    partConnectionsDetail: (passwords: number, tags: number) =>
      `${passwords} with a password, and ${tags} ${tags === 1 ? 'tag' : 'tags'}`,
    partMcp: 'MCP server',
    partMcpDetail: (enabled: boolean, port: number, token: boolean) =>
      `${enabled ? 'On' : 'Off'}, port ${port}${token ? ', with its token' : ''}`,
    partTools: 'pg_dump and psql',
    partToolsDetail: (count: number) => `${count} chosen; only those found on this computer are kept`,
    notInFile: 'Not in this file',
    importButton: 'Import',
    cancel: 'Cancel',
    imported: (added: number, updated: number) => `Imported: ${added} new, ${updated} updated`,
    needPassword: (names: string) => `Without a password, type it in their settings: ${names}.`,
    missingTools: (names: string) => `Not found on this computer, left as they were: ${names}.`,
    failed: 'That didn’t work',
  },
  es: {
    exportTitle: 'Exportar',
    includeSecrets: 'Incluir contraseñas y el token MCP',
    includeSecretsDescription:
      'Se escriben en texto plano: cualquiera con el archivo puede leerlas. Guárdalo en un lugar seguro.',
    exportButton: 'Exportar configuración…',
    exported: (connections: number, passwords: number) =>
      `Se exportaron ${connections} ${connections === 1 ? 'conexión' : 'conexiones'}, ${passwords} con su contraseña`,
    importTitle: 'Importar',
    importHelp:
      'Añade lo que trae el archivo a esta instalación: las conexiones y etiquetas que ya existen se actualizan, no se borra nada.',
    chooseFile: 'Elegir una copia…',
    fromFile: (path: string, when: string, version: string) =>
      `${path} · exportada el ${when} por DBForge ${version}`,
    partPreferences: 'Preferencias',
    partPreferencesDetail: (theme: string, language: string, folder: string | undefined) =>
      `Tema ${theme}, idioma ${language}${folder ? `, exports en ${folder}` : ''}`,
    partConnections: (added: number, updated: number) =>
      `Conexiones: ${added} nuevas, ${updated} actualizadas`,
    partConnectionsDetail: (passwords: number, tags: number) =>
      `${passwords} con contraseña, y ${tags} ${tags === 1 ? 'etiqueta' : 'etiquetas'}`,
    partMcp: 'Servidor MCP',
    partMcpDetail: (enabled: boolean, port: number, token: boolean) =>
      `${enabled ? 'Activado' : 'Desactivado'}, puerto ${port}${token ? ', con su token' : ''}`,
    partTools: 'pg_dump y psql',
    partToolsDetail: (count: number) => `${count} elegidos; solo se conservan los que existen en este equipo`,
    notInFile: 'No viene en este archivo',
    importButton: 'Importar',
    cancel: 'Cancelar',
    imported: (added: number, updated: number) => `Importado: ${added} nuevas, ${updated} actualizadas`,
    needPassword: (names: string) => `Sin contraseña, escríbela en su configuración: ${names}.`,
    missingTools: (names: string) => `No se encontraron en este equipo y quedaron como estaban: ${names}.`,
    failed: 'No se pudo completar',
  },
})
