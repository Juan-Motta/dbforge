import { defineMessages } from './i18n/messages'

/** Texts shared across features: addresses and errors the frontend writes. */
export const messages = defineMessages({
  en: {
    defaultUser: 'default user',
    unexpectedError: 'Unexpected error',
  },
  es: {
    defaultUser: 'usuario por defecto',
    unexpectedError: 'Error inesperado',
  },
})

export type SharedMessages = typeof messages.en
