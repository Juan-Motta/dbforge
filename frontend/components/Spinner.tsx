import styles from './Spinner.module.css'

interface SpinnerProps {
  /** Its width and height, in pixels. */
  size?: number
  className?: string
}

/** A ring turning while something is in progress; decorative, so the text around it says what. */
export function Spinner({ size = 14, className }: SpinnerProps) {
  return (
    <span
      className={className ? `${styles.spinner} ${className}` : styles.spinner}
      style={{ width: size, height: size }}
      aria-hidden
    />
  )
}
