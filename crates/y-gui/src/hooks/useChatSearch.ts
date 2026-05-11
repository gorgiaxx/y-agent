import { useState, useCallback, useEffect, useRef, type RefObject } from 'react';

export interface ChatSearchState {
  isOpen: boolean;
  query: string;
  rawQuery: string;
  currentIndex: number;
  totalMatches: number;
  setTotalMatches: (n: number) => void;
  goToNext: () => void;
  goToPrev: () => void;
  open: () => void;
  close: () => void;
  setQuery: (q: string) => void;
  inputRef: RefObject<HTMLInputElement | null>;
}

export function useChatSearch(): ChatSearchState {
  const [isOpen, setIsOpen] = useState(false);
  const [rawQuery, setRawQuery] = useState('');
  const [query, setDebouncedQuery] = useState('');
  const [currentIndex, setCurrentIndex] = useState(0);
  const [totalMatches, setTotalMatches] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const open = useCallback(() => {
    setIsOpen(true);
  }, []);

  const close = useCallback(() => {
    setIsOpen(false);
    setRawQuery('');
    setDebouncedQuery('');
    setCurrentIndex(0);
    setTotalMatches(0);
  }, []);

  const setQuery = useCallback((q: string) => {
    setRawQuery(q);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      setDebouncedQuery(q);
      setCurrentIndex(0);
    }, 150);
  }, []);

  const goToNext = useCallback(() => {
    setCurrentIndex((prev) => (totalMatches > 0 ? (prev + 1) % totalMatches : 0));
  }, [totalMatches]);

  const goToPrev = useCallback(() => {
    setCurrentIndex((prev) => (totalMatches > 0 ? (prev - 1 + totalMatches) % totalMatches : 0));
  }, [totalMatches]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'f') {
        e.preventDefault();
        if (!isOpen) {
          open();
        } else {
          requestAnimationFrame(() => inputRef.current?.select());
        }
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [isOpen, open]);

  useEffect(() => {
    if (isOpen) {
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [isOpen]);

  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, []);

  return {
    isOpen,
    query,
    rawQuery,
    currentIndex,
    totalMatches,
    setTotalMatches,
    goToNext,
    goToPrev,
    open,
    close,
    setQuery,
    inputRef,
  };
}
