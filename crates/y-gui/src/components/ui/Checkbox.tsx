import * as CheckboxPrimitive from '@radix-ui/react-checkbox'
import { Check } from 'lucide-react'
import { forwardRef } from 'react'

export const Checkbox = forwardRef<HTMLButtonElement, CheckboxPrimitive.CheckboxProps>(
  ({ className = '', ...props }, ref) => {
    return (
      <CheckboxPrimitive.Root
        ref={ref}
        className={[
          'peer inline-flex items-center justify-center align-middle h-[14px] w-[14px] shrink-0 rounded-[3px] border border-solid border-[var(--border)] bg-[var(--surface-primary)]',
          'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--surface-primary)]',
          'disabled:cursor-not-allowed disabled:opacity-50',
          'data-[state=checked]:bg-[var(--accent)] data-[state=checked]:border-[var(--accent)] data-[state=checked]:text-[var(--accent-contrast)]',
          'transition-colors duration-150',
          className,
        ].join(' ')}
        {...props}
      >
        <CheckboxPrimitive.Indicator className="flex items-center justify-center text-current">
          <Check className="h-[10px] w-[10px] stroke-[3]" />
        </CheckboxPrimitive.Indicator>
      </CheckboxPrimitive.Root>
    )
  }
)
Checkbox.displayName = CheckboxPrimitive.Root.displayName
