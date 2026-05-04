import { useEffect, useRef, type RefObject } from 'react';

export function useCloseOnOutsideClick<T extends HTMLElement>(
  ref: RefObject<T | null>,
  open: boolean,
  onClose: () => void,
): void {
  const onCloseRef = useRef(onClose);

  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!open) return;

    const handleOutsideClick = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (ref.current?.contains(target)) return;
      onCloseRef.current();
    };

    document.addEventListener('mousedown', handleOutsideClick);
    return () => document.removeEventListener('mousedown', handleOutsideClick);
  }, [open, ref]);
}
