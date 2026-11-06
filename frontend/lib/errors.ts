/** Backend errors arrive as plain strings; anything else is unexpected. */
export function errorMessage(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return 'Unexpected error'
}
