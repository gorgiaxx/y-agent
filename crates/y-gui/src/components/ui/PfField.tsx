import { type ReactNode } from 'react'

interface PfFieldProps {
  label?: ReactNode
  hint?: string
  full?: boolean
  children: ReactNode
  className?: string
}

export function PfField({ label, hint, full, children, className = '' }: PfFieldProps) {
  return (
    <div className={['pf-field', full && 'pf-field-full', className].filter(Boolean).join(' ')}>
      {label && <label className="pf-label">{label}</label>}
      {children}
      {hint && <span className="pf-hint">{hint}</span>}
    </div>
  )
}
