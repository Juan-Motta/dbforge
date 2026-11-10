import { defineMessages } from './i18n/messages'

/** Texts shared across features: environments, addresses and errors the frontend writes. */
export const messages = defineMessages({
  en: {
    production: 'Production',
    staging: 'Staging',
    development: 'Dev',
    defaultUser: 'default user',
    unexpectedError: 'Unexpected error',
  },
  es: {
    production: 'Producción',
    staging: 'Staging',
    development: 'Desarrollo',
    defaultUser: 'usuario por defecto',
    unexpectedError: 'Error inesperado',
  },
})

export type SharedMessages = typeof messages.en
