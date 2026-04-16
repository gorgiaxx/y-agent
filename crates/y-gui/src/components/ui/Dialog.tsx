import * as DialogPrimitive from '@radix-ui/react-dialog'
import { type ReactNode, forwardRef } from 'react'

/* ---- Root + Trigger (re-exported verbatim) ---- */
export const Dialog = DialogPrimitive.Root
export const DialogTrigger = DialogPrimitive.Trigger
export const DialogClose = DialogPrimitive.Close
export const DialogPortal = DialogPrimitive.Portal

/* ---- Overlay ---- */
export const DialogOverlay = forwardRef<
  HTMLDivElement,
  DialogPrimitive.DialogOverlayProps
>(({ className = '', ...props }, ref) => (
  <DialogPrimitive.Overlay
    ref={ref}
    className={[
      'fixed inset-0 z-9999',
      'flex items-center justify-center',
      'bg-[rgba(0,0,0,0.5)] backdrop-blur-[4px]',
      'data-[state=open]:animate-[dialogOverlayIn_0.15s_ease]',
      className,
    ].join(' ')}
    {...props}
  />
))
DialogOverlay.displayName = 'DialogOverlay'

/* ---- Content ---- */
type DialogSize = 'sm' | 'md' | 'lg' | 'xl'

const dialogSizeMap: Record<DialogSize, string> = {
  sm: '360px',
  md: '480px',
  lg: '640px',
  xl: '720px',
}

interface DialogContentProps extends DialogPrimitive.DialogContentProps {
  /** Predefined size token or raw CSS width value. Default "sm" (360px). */
  size?: DialogSize
  /** @deprecated Use `size` prop instead. Raw CSS width kept for migration. */
  width?: string
  children: ReactNode
}

export const DialogContent = forwardRef<HTMLDivElement, DialogContentProps>(
  ({ className = '', size, width, children, ...props }, ref) => {
    const resolvedWidth = size ? dialogSizeMap[size] : (width ?? dialogSizeMap.sm)

    return (
      <DialogPortal>
        <DialogOverlay>
          <DialogPrimitive.Content
            ref={ref}
            className={[
              'bg-[var(--surface-primary)]',
              'border border-solid border-[var(--border)]',
              'rounded-[var(--radius-lg)]',
              'max-w-[calc(100vw-32px)]',
              'p-6',
              'flex flex-col items-center text-center gap-2',
              'shadow-[0_16px_48px_rgba(0,0,0,0.3),0_0_0_1px_rgba(255,255,255,0.04)]',
              'data-[state=open]:animate-[dialogContentIn_0.2s_cubic-bezier(0.34,1.56,0.64,1)]',
              'outline-none',
              className,
            ].join(' ')}
            style={{ width: resolvedWidth, ...props.style }}
            {...props}
          >
            {children}
          </DialogPrimitive.Content>
        </DialogOverlay>
      </DialogPortal>
    )
  },
)
DialogContent.displayName = 'DialogContent'

/* ---- Title ---- */
export const DialogTitle = forwardRef<
  HTMLHeadingElement,
  DialogPrimitive.DialogTitleProps
>(({ className = '', ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={[
      'm-0 text-15px font-600 text-[var(--text-primary)]',
      className,
    ].join(' ')}
    {...props}
  />
))
DialogTitle.displayName = 'DialogTitle'

/* ---- Description ---- */
export const DialogDescription = forwardRef<
  HTMLParagraphElement,
  DialogPrimitive.DialogDescriptionProps
>(({ className = '', ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={[
      'm-0 text-13px text-[var(--text-secondary)] leading-relaxed',
      className,
    ].join(' ')}
    {...props}
  />
))
DialogDescription.displayName = 'DialogDescription'
