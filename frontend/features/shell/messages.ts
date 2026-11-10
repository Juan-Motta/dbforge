import { defineMessages } from '../../lib/i18n/messages'

export const messages = defineMessages({
  en: {
    // Title bar
    hideExplorer: 'Hide explorer (⌘B)',
    showExplorer: 'Show explorer (⌘B)',
    hideExplorerTitle: 'Hide explorer ⌘B',
    showExplorerTitle: 'Show explorer ⌘B',
    noConnection: 'No connection selected',
    searchEverywhere: 'Search everywhere…',
    selectConnectionFirst: 'Select a connection first',
    newQuery: 'New query',
    settings: 'Settings',
    // Connection details
    connectionDialog: (name: string) => `${name} connection`,
    reachable: 'Reachable',
    notReachable: 'Not reachable',
    checking: 'Checking…',
    editConnection: 'Edit connection',
    // Connection facts
    server: 'Server',
    file: 'File',
    address: 'Address',
    database: 'Database',
    tls: 'TLS',
    tlsOff: 'Off',
    tlsOn: 'On',
    tlsSelfSigned: 'On · self-signed certificate accepted',
  },
  es: {
    hideExplorer: 'Ocultar explorador (⌘B)',
    showExplorer: 'Mostrar explorador (⌘B)',
    hideExplorerTitle: 'Ocultar explorador ⌘B',
    showExplorerTitle: 'Mostrar explorador ⌘B',
    noConnection: 'Ninguna conexión seleccionada',
    searchEverywhere: 'Buscar en todo…',
    selectConnectionFirst: 'Primero selecciona una conexión',
    newQuery: 'Nueva consulta',
    settings: 'Configuración',
    connectionDialog: (name: string) => `Conexión ${name}`,
    reachable: 'Accesible',
    notReachable: 'No accesible',
    checking: 'Comprobando…',
    editConnection: 'Editar conexión',
    server: 'Servidor',
    file: 'Archivo',
    address: 'Dirección',
    database: 'Base de datos',
    tls: 'TLS',
    tlsOff: 'Desactivado',
    tlsOn: 'Activado',
    tlsSelfSigned: 'Activado · se acepta un certificado autofirmado',
  },
})

export type ShellMessages = typeof messages.en
