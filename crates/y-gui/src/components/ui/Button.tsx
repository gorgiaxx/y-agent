import { type ButtonHTMLAttributes, forwardRef } from 'react'

type ButtonVariant = 'primary' | 'ghost' | 'danger' | 'warning' | 'outline' | 'icon'
type ButtonSize = 'sm' | 'md'

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant
  size?: ButtonSize
}

const variantStyles: Record<ButtonVariant, string> = {
  primary:
    'bg-accent text-accent-contrast border-transparent hover:op-85',
  ghost:
    'bg-transparent text-text-secondary border-border hover:(bg-surface-hover text-text-primary)',
  danger:
    'bg-error text-white border-transparent hover:op-85',
  warning:
    'bg-[var(--warning)] text-[#1a1917] border-transparent hover:op-85',
  outline:
    'bg-surface-primary text-text-secondary border-border hover:(bg-surface-hover text-text-primary)',
  icon:
    'bg-transparent text-text-muted border-transparent hover:(text-text-primary border-border bg-surface-hover)',
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
          'rounded-md',
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
