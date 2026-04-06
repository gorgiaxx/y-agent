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
    className={className}
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
    className={className}
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
