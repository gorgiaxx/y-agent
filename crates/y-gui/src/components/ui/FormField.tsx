import { type ReactNode } from 'react'

interface FormFieldProps {
  label: string
  hint?: string
  children: ReactNode
  className?: string
}

export function FormField({ label, hint, children, className = '' }: FormFieldProps) {
  return (
    <div className={['form-group', className].filter(Boolean).join(' ')}>
      <label className="form-label">{label}</label>
      {children}
      {hint && <p className="form-hint">{hint}</p>}
    </div>
  )
}
