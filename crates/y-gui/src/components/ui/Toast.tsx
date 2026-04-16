import * as ToastPrimitive from '@radix-ui/react-toast'
import { useState, useCallback, useRef, type ReactNode } from 'react'
import { ToastContext, type ToastType } from '../../hooks/useToast'

interface ToastData {
  id: number
  message: string
  type: ToastType
}

/* ---- Viewport (renders at bottom-center) ---- */
const typeStyles: Record<ToastType, string> = {
  success: 'bg-[rgba(111,207,151,0.12)] border-[rgba(111,207,151,0.25)] text-[var(--success)]',
  error: 'bg-[var(--error-subtle)] border-[rgba(229,115,115,0.2)] text-[var(--error)]',
  info: 'bg-[var(--surface-tertiary)] border-[var(--border)] text-[var(--text-primary)]',
}

/* ---- Provider ---- */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastData[]>([])
  const nextIdRef = useRef(0)

  const addToast = useCallback((message: string, type: ToastType = 'success') => {
    const id = ++nextIdRef.current
    setToasts((prev) => [...prev, { id, message, type }])
    // Auto-dismiss
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id))
    }, 3000)
  }, [])

  return (
    <ToastContext.Provider value={{ toast: addToast }}>
      <ToastPrimitive.Provider swipeDirection="down" duration={3000}>
        {children}
        {toasts.map((t) => (
          <ToastPrimitive.Root
            key={t.id}
            className={[
              'py-1.5 px-4',
              'text-12px font-500',
              'rounded-[var(--radius-md)]',
              'whitespace-nowrap',
              'shadow-md',
              'border border-solid',
              'animate-[toastIn_0.25s_ease]',
              typeStyles[t.type],
            ].join(' ')}
            onOpenChange={(open) => {
              if (!open) setToasts((prev) => prev.filter((x) => x.id !== t.id))
            }}
          >
            <ToastPrimitive.Description>{t.message}</ToastPrimitive.Description>
          </ToastPrimitive.Root>
        ))}
        <ToastPrimitive.Viewport
          className={[
            'fixed bottom-4 left-1/2 -translate-x-1/2',
            'flex flex-col items-center gap-2',
            'z-[1010]',
            'outline-none',
          ].join(' ')}
        />
      </ToastPrimitive.Provider>
    </ToastContext.Provider>
  )
}
