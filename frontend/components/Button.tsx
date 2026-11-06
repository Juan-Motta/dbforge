import type { ButtonHTMLAttributes } from 'react'
import styles from './Button.module.css'

type Variant = 'default' | 'primary' | 'ghost'

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant
}

export function Button({ variant = 'default', className, ...rest }: ButtonProps) {
  const classes = [styles.button, variant !== 'default' && styles[variant], className]
  return <button type="button" className={classes.filter(Boolean).join(' ')} {...rest} />
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string
  active?: boolean
}

/** Square, borderless button for toolbars. `label` is required for accessibility. */
export function IconButton({ label, active = false, className, ...rest }: IconButtonProps) {
  const classes = [styles.button, styles.icon, active && styles.active, className]
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={active}
      className={classes.filter(Boolean).join(' ')}
      {...rest}
    />
  )
}
