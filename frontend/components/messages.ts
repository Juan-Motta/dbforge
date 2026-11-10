import { defineMessages } from '../lib/i18n/messages'

export const messages = defineMessages({
  en: {
    close: 'Close',
    closeTab: (title: string) => `Close ${title}`,
    resetHint: (label: string) => `${label} · double-click to reset`,
  },
  es: {
    close: 'Cerrar',
    closeTab: (title: string) => `Cerrar ${title}`,
    resetHint: (label: string) => `${label} · doble clic para restablecer`,
  },
})
