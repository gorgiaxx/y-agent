import * as ScrollAreaPrimitive from '@radix-ui/react-scroll-area'
import { forwardRef, type ReactNode } from 'react'

interface ScrollAreaProps extends ScrollAreaPrimitive.ScrollAreaProps {
  children: ReactNode
}

export const ScrollArea = forwardRef<HTMLDivElement, ScrollAreaProps>(
  ({ className = '', children, ...props }, ref) => (
    <ScrollAreaPrimitive.Root
      ref={ref}
      className={['overflow-hidden', className].join(' ')}
      {...props}
    >
      <ScrollAreaPrimitive.Viewport className="w-full h-full">
        {children}
      </ScrollAreaPrimitive.Viewport>
      <ScrollAreaPrimitive.Scrollbar
        orientation="vertical"
        className={[
          'flex touch-none select-none',
          'w-1 p-[1px]',
          'transition-colors duration-150',
        ].join(' ')}
      >
        <ScrollAreaPrimitive.Thumb
          className={[
            'relative flex-1',
            'rounded-[2px]',
            'bg-[rgba(255,255,255,0.10)]',
            'hover:bg-[rgba(255,255,255,0.18)]',
          ].join(' ')}
        />
      </ScrollAreaPrimitive.Scrollbar>
      <ScrollAreaPrimitive.Corner />
    </ScrollAreaPrimitive.Root>
  ),
)

ScrollArea.displayName = 'ScrollArea'
