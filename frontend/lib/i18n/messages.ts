/** A text, or a function that writes one from values such as a name or a count. */
export type Message = string | ((...values: never[]) => string)

/**
 * A feature's texts in both languages. Spanish must carry every English
 * message, each with the same parameters, or the type checker refuses it.
 */
export function defineMessages<T extends Record<string, Message>>(messages: { en: T; es: NoInfer<T> }) {
  return messages
}
