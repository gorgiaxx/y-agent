import { useState, useRef, useCallback, useEffect } from 'react';

export function useSidebarSearch() {
  const [searchQuery, setSearchQuery] = useState('');
  const [searchOpen, setSearchOpen] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (searchOpen) {
      requestAnimationFrame(() => searchInputRef.current?.focus());
    }
  }, [searchOpen]);

  const closeSearch = useCallback(() => {
    setSearchQuery('');
    setSearchOpen(false);
  }, []);

  return {
    searchQuery,
    setSearchQuery,
    searchOpen,
    setSearchOpen,
    searchInputRef,
    closeSearch,
  };
}
