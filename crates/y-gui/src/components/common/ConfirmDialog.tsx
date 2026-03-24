import { useEffect, useRef } from 'react';
import { AlertTriangle, Loader2 } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  Button,
} from '../ui';

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  variant?: 'danger' | 'warning';
  loading?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

const iconBgMap = {
  danger: 'bg-[var(--error-subtle,rgba(239,68,68,0.12))] text-[var(--error,#f87171)]',
  warning: 'bg-[rgba(251,191,36,0.12)] text-[#fbbf24]',
};

const confirmBtnMap = {
  danger: 'danger' as const,
  warning: 'primary' as const,
};

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel = 'Delete',
  cancelLabel = 'Cancel',
  variant = 'danger',
  loading = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const cancelBtnRef = useRef<HTMLButtonElement>(null);

  // Focus cancel button when dialog opens
  useEffect(() => {
    if (!open) return;
    // Slight delay so Radix finishes mounting
    const raf = requestAnimationFrame(() => cancelBtnRef.current?.focus());
    return () => cancelAnimationFrame(raf);
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={(isOpen) => { if (!isOpen && !loading) onCancel(); }}>
      <DialogContent width="360px">
        {/* Icon */}
        <div
          className={[
            'w-11 h-11 rounded-full',
            'flex items-center justify-center',
            'mb-1',
            iconBgMap[variant],
          ].join(' ')}
        >
          <AlertTriangle size={22} />
        </div>

        <DialogTitle>{title}</DialogTitle>
        <DialogDescription>{message}</DialogDescription>

        {/* Actions */}
        <div className="flex gap-2 w-full mt-2">
          <Button
            ref={cancelBtnRef}
            variant="ghost"
            className="flex-1"
            onClick={onCancel}
            disabled={loading}
          >
            {cancelLabel}
          </Button>
          <Button
            variant={confirmBtnMap[variant]}
            className={[
              'flex-1',
              variant === 'warning' ? 'bg-[#f59e0b] hover:bg-[#d97706] text-white border-transparent' : '',
            ].join(' ')}
            onClick={onConfirm}
            disabled={loading}
          >
            {loading ? (
              <span className="inline-flex items-center gap-1.5">
                <Loader2 size={14} className="animate-spin" />
                Deleting...
              </span>
            ) : (
              confirmLabel
            )}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
