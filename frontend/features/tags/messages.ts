import { defineMessages } from '../../lib/i18n/messages'
import type { TagColor } from './tags'

export const messages = defineMessages({
  en: {
    tags: 'Tags',
    tagsIntro: 'Mark connections by environment. Each tag has a color and can ask before changes.',
    name: (tag: string) => `Name of ${tag}`,
    color: (tag: string) => `Color of ${tag}`,
    colorName: (color: TagColor) =>
      ({
        red: 'Red',
        orange: 'Orange',
        amber: 'Amber',
        green: 'Green',
        teal: 'Teal',
        blue: 'Blue',
        violet: 'Violet',
        pink: 'Pink',
        gray: 'Gray',
      })[color],
    confirm: 'Ask before changes',
    confirmHelp: 'Editing data or importing on these connections asks first.',
    remove: (tag: string) => `Delete ${tag}`,
    add: 'Add tag',
    newName: (n: number) => (n === 1 ? 'NEW' : `NEW ${n}`),
  },
  es: {
    tags: 'Etiquetas',
    tagsIntro:
      'Marca las conexiones por entorno. Cada etiqueta tiene un color y puede pedir confirmación antes de cambios.',
    name: (tag: string) => `Nombre de ${tag}`,
    color: (tag: string) => `Color de ${tag}`,
    colorName: (color: TagColor) =>
      ({
        red: 'Rojo',
        orange: 'Naranja',
        amber: 'Ámbar',
        green: 'Verde',
        teal: 'Verde azulado',
        blue: 'Azul',
        violet: 'Violeta',
        pink: 'Rosa',
        gray: 'Gris',
      })[color],
    confirm: 'Confirmar antes de cambios',
    confirmHelp: 'Editar datos o importar en estas conexiones pide confirmación.',
    remove: (tag: string) => `Eliminar ${tag}`,
    add: 'Agregar etiqueta',
    newName: (n: number) => (n === 1 ? 'NUEVA' : `NUEVA ${n}`),
  },
})

export type TagsMessages = typeof messages.en
