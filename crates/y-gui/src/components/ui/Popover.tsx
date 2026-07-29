import * as PopoverPrimitive from '@radix-ui/react-popover'
import { forwardRef, type ReactNode } from 'react'

/* ---- Root (re-exported verbatim) ---- */
export const Popover = PopoverPrimitive.Root

/* ---- Trigger ---- */
export const PopoverTrigger = PopoverPrimitive.Trigger

/* ---- Content ---- */
interface PopoverContentProps extends PopoverPrimitive.PopoverContentProps {
  children: ReactNode
}

export const PopoverContent = forwardRef<HTMLDivElement, PopoverContentProps>(
  ({ className = '', children, sideOffset = 4, align = 'start', ...props }, ref) => (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        ref={ref}
        sideOffset={sideOffset}
        align={align}
        className={[
          'z-100',
          'bg-surface-primary',
          'border border-solid border-border',
          'rounded-md',
          'shadow-md',
          'animate-[selectIn_0.1s_ease-out]',
          'outline-none',
          className,
        ].join(' ')}
        {...props}
      >
        {children}
      </PopoverPrimitive.Content>
    </PopoverPrimitive.Portal>
  ),
)
PopoverContent.displayName = 'PopoverContent'
