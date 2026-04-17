import { type ReactNode } from 'react'

interface PfSectionProps {
  title: ReactNode
  className?: string
}

export function PfSection({ title, className = '' }: PfSectionProps) {
  return (
    <div className={['pf-section-divider', className].filter(Boolean).join(' ')}>
      <span className="pf-section-title">{title}</span>
    </div>
  )
}
