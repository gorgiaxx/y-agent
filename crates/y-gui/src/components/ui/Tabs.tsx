import * as TabsPrimitive from '@radix-ui/react-tabs'
import { forwardRef } from 'react'

/* ---- Root (re-exported verbatim) ---- */
export const Tabs = TabsPrimitive.Root

/* ---- List ---- */
export const TabsList = forwardRef<
  HTMLDivElement,
  TabsPrimitive.TabsListProps
>(({ className = '', ...props }, ref) => (
  <TabsPrimitive.List
    ref={ref}
    className={[
      'inline-flex items-center gap-0.5',
      'bg-surface-secondary',
      'border border-solid border-border',
      'rounded-[var(--radius-md)]',
      'p-[3px]',
    ].join(' ')}
    {...props}
  />
))
TabsList.displayName = 'TabsList'

/* ---- Trigger ---- */
export const TabsTrigger = forwardRef<
  HTMLButtonElement,
  TabsPrimitive.TabsTriggerProps
>(({ className = '', ...props }, ref) => (
  <TabsPrimitive.Trigger
    ref={ref}
    className={[
      'flex-1 inline-flex items-center justify-center',
      'px-3.5 py-1.5',
      'bg-transparent border-none',
      'rounded-[6px]',
      'text-13px font-500',
      'text-text-secondary',
      'cursor-pointer outline-none',
      'transition-all duration-150',
      'hover:(text-text-primary bg-surface-hover)',
      'data-[state=active]:(bg-accent text-accent-contrast)',
    ].join(' ')}
    {...props}
  />
))
TabsTrigger.displayName = 'TabsTrigger'

/* ---- Content ---- */
export const TabsContent = forwardRef<
  HTMLDivElement,
  TabsPrimitive.TabsContentProps
>(({ className = '', ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    className={className}
    {...props}
  />
))
TabsContent.displayName = 'TabsContent'
