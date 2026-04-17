import { type ReactNode } from 'react'

interface SettingsItemProps {
  title: ReactNode
  description?: ReactNode
  children: ReactNode
  /** Use a wider layout for inputs that need more horizontal space (e.g. text inputs, tag inputs) */
  wide?: boolean
  className?: string
}

export function SettingsItem({ title, description, children, wide = false, className = '' }: SettingsItemProps) {
  return (
    <div className={['settings-item', wide && 'settings-item--wide', !description && 'settings-item--no-desc', className].filter(Boolean).join(' ')}>
      <div className="settings-item-label">
        <div className="settings-item-title">{title}</div>
        {description && <div className="settings-item-description">{description}</div>}
      </div>
      <div className="settings-item-control">
        {children}
      </div>
    </div>
  )
}
