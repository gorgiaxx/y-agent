import * as SeparatorPrimitive from '@radix-ui/react-separator'
import { forwardRef } from 'react'

export const Separator = forwardRef<
  HTMLDivElement,
  SeparatorPrimitive.SeparatorProps
>(({ className = '', orientation = 'horizontal', ...props }, ref) => (
  <SeparatorPrimitive.Root
    ref={ref}
    orientation={orientation}
    className={[
      'shrink-0 bg-[var(--border)]',
      orientation === 'horizontal' ? 'h-px w-full' : 'w-px h-full',
      className,
    ].join(' ')}
    {...props}
  />
))

Separator.displayName = 'Separator'
