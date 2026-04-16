import { type InputHTMLAttributes, forwardRef } from 'react'

interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  /** Visual variant */
  variant?: 'default' | 'mono'
  /** Use tabular-nums for numeric fields */
  numeric?: boolean
}

export const Input = forwardRef<HTMLInputElement, InputProps>(
  ({ variant = 'default', numeric = false, className = '', ...props }, ref) => {
    const monoClass = variant === 'mono'
      ? "font-[SF_Mono,Fira_Code,Consolas,monospace]"
      : 'font-sans'

    const numClass = numeric ? 'font-variant-numeric-tabular-nums' : ''

    return (
      <input
        ref={ref}
        className={[
          'w-full',
          'px-3 py-1.5',
          'text-12px',
          monoClass,
          numClass,
          'border border-solid border-[var(--border)]',
          'rounded-[var(--radius-md)]',
          'bg-[var(--surface-primary)]',
          'text-[var(--text-primary)]',
          'outline-none',
          'transition-colors duration-150',
          'focus:border-[var(--border-focus)]',
          'placeholder:text-[var(--text-muted)]',
          className,
        ].join(' ')}
        {...props}
      />
    )
  },
)

Input.displayName = 'Input'

/* ---- Textarea variant ---- */

import { type TextareaHTMLAttributes } from 'react'

interface TextareaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> {
  variant?: 'default' | 'mono'
}

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ variant = 'default', className = '', ...props }, ref) => {
    const monoClass = variant === 'mono'
      ? "font-[SF_Mono,Fira_Code,Consolas,monospace]"
      : 'font-sans'

    return (
      <textarea
        ref={ref}
        className={[
          'w-full',
          'px-3 py-2',
          'text-12px',
          monoClass,
          'leading-[1.65]',
          'border border-solid border-[var(--border)]',
          'rounded-[var(--radius-md)]',
          'bg-[var(--surface-primary)]',
          'text-[var(--text-primary)]',
          'outline-none',
          'transition-colors duration-150',
          'resize-y',
          'tab-size-2',
          'focus:border-[var(--border-focus)]',
          className,
        ].join(' ')}
        {...props}
      />
    )
  },
)

Textarea.displayName = 'Textarea'
