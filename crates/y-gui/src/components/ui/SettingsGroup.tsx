import { type ReactNode } from 'react'

interface SettingsGroupProps {
  title: ReactNode
  description?: ReactNode
  children: ReactNode
  className?: string
}

export function SettingsGroup({ title, description, children, className = '' }: SettingsGroupProps) {
  return (
    <div className={['settings-group', className].filter(Boolean).join(' ')}>
      <div className="settings-group-header">
        <div className="settings-group-title">{title}</div>
        {description && <div className="settings-group-description">{description}</div>}
      </div>
      <div className="settings-group-body">
        {children}
      </div>
    </div>
  )
}
