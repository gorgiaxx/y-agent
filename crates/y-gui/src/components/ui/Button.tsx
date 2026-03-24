import { type ButtonHTMLAttributes, forwardRef } from 'react'

type ButtonVariant = 'primary' | 'ghost' | 'danger' | 'outline' | 'icon'
type ButtonSize = 'sm' | 'md'

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant
  size?: ButtonSize
}

const variantStyles: Record<ButtonVariant, string> = {
  primary:
    'bg-[var(--accent)] text-[#0f0f0f] border-transparent hover:op-85',
  ghost:
    'bg-transparent text-[var(--text-secondary)] border-[var(--border)] hover:(bg-[var(--surface-hover)] text-[var(--text-primary)])',
  danger:
    'bg-[var(--error)] text-white border-transparent hover:op-85',
  outline:
    'bg-[var(--surface-primary)] text-[var(--text-secondary)] border-[var(--border)] hover:(bg-[var(--surface-hover)] text-[var(--text-primary)])',
  icon:
    'bg-transparent text-[var(--text-muted)] border-transparent hover:(text-[var(--text-primary)] border-[var(--border)] bg-[var(--surface-hover)])',
}

const sizeStyles: Record<ButtonSize, string> = {
  sm: 'px-3 py-1 text-11px h-7',
  md: 'px-4 py-1.5 text-12px h-8',
}

const iconSizeStyles: Record<ButtonSize, string> = {
  sm: 'w-7 h-7',
  md: 'w-8 h-8',
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ variant = 'ghost', size = 'md', className = '', disabled, ...props }, ref) => {
    const isIcon = variant === 'icon'
    const sizeClass = isIcon ? iconSizeStyles[size] : sizeStyles[size]

    return (
      <button
        ref={ref}
        className={[
          'inline-flex items-center justify-center gap-1',
          'font-500 font-sans cursor-pointer',
          'rounded-[var(--radius-md)]',
          'border border-solid',
          'transition-all duration-150',
          'outline-none',
          'disabled:(op-55 cursor-not-allowed pointer-events-none)',
          variantStyles[variant],
          sizeClass,
          className,
        ].join(' ')}
        disabled={disabled}
        {...props}
      />
    )
  },
)

Button.displayName = 'Button'
