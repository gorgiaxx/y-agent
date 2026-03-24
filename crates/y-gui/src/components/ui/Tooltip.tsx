import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import { type ReactNode, forwardRef } from 'react'

/* ---- Provider (mount once at app root) ---- */
export const TooltipProvider = TooltipPrimitive.Provider

/* ---- Composable API ---- */
export const TooltipRoot = TooltipPrimitive.Root
export const TooltipTrigger = TooltipPrimitive.Trigger

export const TooltipContent = forwardRef<
  HTMLDivElement,
  TooltipPrimitive.TooltipContentProps
>(({ className = '', sideOffset = 6, ...props }, ref) => (
  <TooltipPrimitive.Portal>
    <TooltipPrimitive.Content
      ref={ref}
      sideOffset={sideOffset}
      className={[
        'px-2.5 py-1',
        'bg-[var(--surface-primary)]',
        'border border-solid border-[var(--border)]',
        'rounded-[var(--radius-sm)]',
        'text-11px text-[var(--text-secondary)]',
        'whitespace-nowrap',
        'shadow-[0_4px_12px_rgba(0,0,0,0.2)]',
        'z-300',
        'animate-[tooltipIn_0.15s_ease]',
        className,
      ].join(' ')}
      {...props}
    />
  </TooltipPrimitive.Portal>
))
TooltipContent.displayName = 'TooltipContent'

/* ---- Simple convenience wrapper ---- */
interface TooltipProps {
  content: ReactNode
  children: ReactNode
  side?: 'top' | 'right' | 'bottom' | 'left'
  delayDuration?: number
}

export function Tooltip({ content, children, side = 'top', delayDuration = 300 }: TooltipProps) {
  return (
    <TooltipRoot delayDuration={delayDuration}>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side={side}>{content}</TooltipContent>
    </TooltipRoot>
  )
}
