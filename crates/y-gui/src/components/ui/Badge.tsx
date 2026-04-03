import { type HTMLAttributes, forwardRef } from 'react'
import { X } from 'lucide-react'

type BadgeVariant = 'default' | 'accent' | 'success' | 'error' | 'outline'

interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  variant?: BadgeVariant
  /** If true, renders a small "x" dismiss button */
  onDismiss?: () => void
}

const variantStyles: Record<BadgeVariant, string> = {
  default:
    'bg-[var(--surface-hover)] text-[var(--text-primary)] border-[var(--border)]',
  accent:
    'bg-[var(--accent-subtle)] text-[var(--accent)] border-[rgba(200,181,96,0.25)]',
  success:
    'bg-[rgba(111,207,151,0.08)] text-[var(--success)] border-[rgba(111,207,151,0.2)]',
  error:
    'bg-[var(--error-subtle)] text-[var(--error)] border-[rgba(229,115,115,0.2)]',
  outline:
    'bg-transparent text-[var(--text-secondary)] border-[var(--border)]',
}

export const Badge = forwardRef<HTMLSpanElement, BadgeProps>(
  ({ variant = 'default', onDismiss, className = '', children, ...props }, ref) => (
    <span
      ref={ref}
      className={[
        'inline-flex items-center gap-1',
        'px-1.5 py-0',
        'text-9px font-500',
        'leading-4',
        'whitespace-nowrap',
        'rounded-full',
        'border border-solid',
        'tracking-[0.04em]',
        variantStyles[variant],
        className,
      ].join(' ')}
      {...props}
    >
      <span className="overflow-hidden text-ellipsis">{children}</span>
      {onDismiss && (
        <button
          type="button"
          onClick={(e) => { e.stopPropagation(); onDismiss(); }}
          className={[
            'inline-flex items-center justify-center',
            'w-3 h-3',
            'border-none bg-none p-0',
            'rounded-sm',
            'text-inherit op-60',
            'cursor-pointer',
            'transition-all duration-100',
            'hover:(op-100 text-[var(--error)] bg-[var(--error-subtle)])',
            'text-11px leading-none',
          ].join(' ')}
        >
          <X className="w-2.5 h-2.5" />
        </button>
      )}
    </span>
  ),
)

Badge.displayName = 'Badge'
