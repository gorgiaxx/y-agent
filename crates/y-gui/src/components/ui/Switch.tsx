import * as SwitchPrimitive from '@radix-ui/react-switch'
import { forwardRef } from 'react'

interface SwitchProps extends SwitchPrimitive.SwitchProps {
  label?: string
}

export const Switch = forwardRef<HTMLButtonElement, SwitchProps>(
  ({ label, className = '', ...props }, ref) => {
    const switchEl = (
      <SwitchPrimitive.Root
        ref={ref}
        className={[
          'relative inline-flex items-center',
          'w-9 h-5',
          'rounded-full',
          'border border-solid border-[var(--border)]',
          'bg-[var(--surface-tertiary)]',
          'cursor-pointer',
          'transition-colors duration-200',
          'data-[state=checked]:(bg-[var(--accent)] border-[var(--accent)])',
          'focus-visible:(outline-2 outline-offset-2 outline-[var(--accent)])',
          'disabled:(op-50 cursor-not-allowed)',
          className,
        ].join(' ')}
        {...props}
      >
        <SwitchPrimitive.Thumb
          className={[
            'block w-3.5 h-3.5',
            'rounded-full',
            'bg-white',
            'transition-transform duration-200',
            'translate-x-0.75',
            'data-[state=checked]:translate-x-[18px]',
            'shadow-sm',
          ].join(' ')}
        />
      </SwitchPrimitive.Root>
    )

    if (label) {
      return (
        <label className="inline-flex items-center gap-2 cursor-pointer text-13px text-[var(--text-primary)]">
          {switchEl}
          <span>{label}</span>
        </label>
      )
    }

    return switchEl
  },
)

Switch.displayName = 'Switch'
