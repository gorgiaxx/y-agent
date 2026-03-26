import * as SelectPrimitive from '@radix-ui/react-select'
import { forwardRef } from 'react'
import { ChevronDown, Check } from 'lucide-react'

/* ---- Root re-exports ---- */
export const Select = SelectPrimitive.Root
export const SelectGroup = SelectPrimitive.Group

export const SelectValue = forwardRef<
  HTMLSpanElement,
  SelectPrimitive.SelectValueProps
>(({ className = '', style, ...props }, ref) => (
  <SelectPrimitive.Value
    ref={ref}
    className={className}
    style={{
      overflow: 'hidden',
      whiteSpace: 'nowrap',
      textOverflow: 'ellipsis',
      flex: '1 1 0%',
      minWidth: 0,
      textAlign: 'left',
      ...style,
    }}
    {...props}
  />
))
SelectValue.displayName = 'SelectValue'

/* ---- Trigger ---- */
export const SelectTrigger = forwardRef<
  HTMLButtonElement,
  SelectPrimitive.SelectTriggerProps
>(({ className = '', children, style, ...props }, ref) => (
  <SelectPrimitive.Trigger
    ref={ref}
    className={[
      'inline-flex items-center justify-between gap-2',
      'w-full min-w-0',
      'px-2 py-1.5',
      'text-12px font-sans',
      'border border-solid border-[var(--border)]',
      'rounded-[var(--radius-md)]',
      'bg-[var(--surface-primary)]',
      'text-[var(--text-primary)]',
      'cursor-pointer outline-none',
      'transition-colors duration-150',
      'focus:border-[var(--border-focus)]',
      'data-[state=open]:border-[var(--border-focus)]',
      className,
    ].join(' ')}
    style={{ overflow: 'hidden', whiteSpace: 'nowrap', ...style }}
    {...props}
  >
    {children}
    <SelectPrimitive.Icon className="flex-shrink-0">
      <ChevronDown size={12} className="text-[var(--text-muted)] op-70" />
    </SelectPrimitive.Icon>
  </SelectPrimitive.Trigger>
))
SelectTrigger.displayName = 'SelectTrigger'

/* ---- Content ---- */
export const SelectContent = forwardRef<
  HTMLDivElement,
  SelectPrimitive.SelectContentProps
>(({ className = '', children, position = 'popper', ...props }, ref) => (
  <SelectPrimitive.Portal>
    <SelectPrimitive.Content
      ref={ref}
      position={position}
      className={[
        'overflow-hidden',
        'bg-[var(--surface-primary)]',
        'border border-solid border-[var(--border)]',
        'rounded-[var(--radius-md)]',
        'shadow-[0_8px_24px_rgba(0,0,0,0.25)]',
        'z-200',
        'min-w-[var(--radix-select-trigger-width)]',
        'max-h-[300px]',
        'animate-[selectIn_0.1s_ease-out]',
        className,
      ].join(' ')}
      {...props}
    >
      <SelectPrimitive.Viewport className="p-1">
        {children}
      </SelectPrimitive.Viewport>
    </SelectPrimitive.Content>
  </SelectPrimitive.Portal>
))
SelectContent.displayName = 'SelectContent'

/* ---- Item ---- */
export const SelectItem = forwardRef<
  HTMLDivElement,
  SelectPrimitive.SelectItemProps
>(({ className = '', children, ...props }, ref) => (
  <SelectPrimitive.Item
    ref={ref}
    className={[
      'relative flex items-center',
      'px-2 py-1.5 pl-7',
      'text-12px font-sans',
      'rounded-[var(--radius-sm)]',
      'text-[var(--text-secondary)]',
      'cursor-pointer outline-none',
      'transition-colors duration-100',
      'data-[highlighted]:(bg-[var(--surface-hover)] text-[var(--text-primary)])',
      className,
    ].join(' ')}
    {...props}
  >
    <SelectPrimitive.ItemIndicator className="absolute left-1.5 flex items-center justify-center">
      <Check size={12} className="text-[var(--accent)]" />
    </SelectPrimitive.ItemIndicator>
    <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
  </SelectPrimitive.Item>
))
SelectItem.displayName = 'SelectItem'

/* ---- Label ---- */
export const SelectLabel = forwardRef<
  HTMLDivElement,
  SelectPrimitive.SelectLabelProps
>(({ className = '', ...props }, ref) => (
  <SelectPrimitive.Label
    ref={ref}
    className={[
      'px-2 py-1',
      'text-10px font-500',
      'text-[var(--text-muted)]',
      'uppercase tracking-[0.06em]',
      className,
    ].join(' ')}
    {...props}
  />
))
SelectLabel.displayName = 'SelectLabel'

/* ---- Separator ---- */
export const SelectSeparator = forwardRef<
  HTMLDivElement,
  SelectPrimitive.SelectSeparatorProps
>(({ className = '', ...props }, ref) => (
  <SelectPrimitive.Separator
    ref={ref}
    className={['h-px my-1 bg-[var(--border)]', className].join(' ')}
    {...props}
  />
))
SelectSeparator.displayName = 'SelectSeparator'
