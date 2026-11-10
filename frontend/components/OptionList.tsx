import type { HTMLAttributes, InputHTMLAttributes, ReactNode } from 'react'
import { Icon, type IconName } from './Icon'
import styles from './OptionList.module.css'

/** Choices as one bordered list, a row each; `role="radiogroup"` and a label go on it for radios. */
export function OptionList({ children, ...rest }: { children: ReactNode } & HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={styles.list} {...rest}>
      {children}
    </div>
  )
}

interface OptionProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'type' | 'title'> {
  type: 'radio' | 'checkbox'
  title: ReactNode
  description?: ReactNode
  icon?: IconName
}

/**
 * One row of an OptionList: a radio or checkbox drawn in the app's colors
 * (never the system's blue focus ring), an optional icon, a title and a line
 * saying what it does.
 */
export function Option({ type, title, description, icon, ...input }: OptionProps) {
  return (
    <label className={styles.option}>
      <input type={type} {...input} />
      {icon && (
        <span className={styles.icon}>
          <Icon name={icon} size={15} />
        </span>
      )}
      <span className={styles.text}>
        <span className={styles.title}>{title}</span>
        {description && <span className={styles.description}>{description}</span>}
      </span>
    </label>
  )
}
