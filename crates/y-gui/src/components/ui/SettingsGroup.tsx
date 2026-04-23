import { type ReactNode } from 'react'

interface SettingsGroupProps {
  title: ReactNode
  description?: ReactNode
  children: ReactNode
  className?: string
  bodyVariant?: 'card' | 'plain'
}

export function SettingsGroup({
  title,
  description,
  children,
  className = '',
  bodyVariant = 'card',
}: SettingsGroupProps) {
  const bodyClassName = [
    'settings-group-body',
    bodyVariant === 'plain' ? 'settings-group-body--plain' : '',
  ].filter(Boolean).join(' ')

  return (
    <div className={['settings-group', className].filter(Boolean).join(' ')}>
      <div className="settings-group-header">
        <div className="settings-group-title">{title}</div>
        {description && <div className="settings-group-description">{description}</div>}
      </div>
      <div className={bodyClassName}>
        {children}
      </div>
    </div>
  )
}
