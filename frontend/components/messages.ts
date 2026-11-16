import { defineMessages } from '../lib/i18n/messages'

export const messages = defineMessages({
  en: {
    close: 'Close',
    closeTab: (title: string) => `Close ${title}`,
    resetHint: (label: string) => `${label} · double-click to reset`,
    showError: 'Show the whole error',
    errorTitle: 'Error',
    copy: 'Copy',
  },
  es: {
    close: 'Cerrar',
    closeTab: (title: string) => `Cerrar ${title}`,
    resetHint: (label: string) => `${label} · doble clic para restablecer`,
    showError: 'Ver el error completo',
    errorTitle: 'Error',
    copy: 'Copiar',
  },
})
