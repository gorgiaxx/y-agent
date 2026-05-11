import { useCallback, useLayoutEffect, useRef, type RefObject } from 'react';
import { Search, ChevronUp, ChevronDown, X } from 'lucide-react';
import { useChatSearchContext } from '../../hooks/useChatSearchContext';
import './ChatSearchToolbar.css';

interface ChatSearchToolbarProps {
  scrollContainerRef: RefObject<HTMLDivElement | null>;
}

export function ChatSearchToolbar({ scrollContainerRef }: ChatSearchToolbarProps) {
  const {
    isOpen,
    query,
    rawQuery,
    currentIndex,
    totalMatches,
    setTotalMatches,
    goToNext,
    goToPrev,
    close,
    setQuery,
    inputRef,
  } = useChatSearchContext();
  const prevActiveRef = useRef<Element | null>(null);

  const handleInputChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      setQuery(e.target.value);
    },
    [setQuery],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        close();
      } else if (e.key === 'Enter' && e.shiftKey) {
        e.preventDefault();
        goToPrev();
      } else if (e.key === 'Enter') {
        e.preventDefault();
        goToNext();
      }
    },
    [close, goToPrev, goToNext],
  );

  useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container || !query) {
      if (totalMatches !== 0) setTotalMatches(0);
      return;
    }

    const marks = container.querySelectorAll('[data-search-match]');
    if (marks.length !== totalMatches) {
      setTotalMatches(marks.length);
    }
  });

  useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    if (prevActiveRef.current) {
      prevActiveRef.current.classList.remove('chat-search-match--active');
      prevActiveRef.current = null;
    }

    if (totalMatches === 0 || !query) return;

    const marks = container.querySelectorAll('[data-search-match]');
    const idx = Math.min(currentIndex, marks.length - 1);
    const active = marks[idx];
    if (active) {
      active.classList.add('chat-search-match--active');
      active.scrollIntoView({ block: 'center', behavior: 'smooth' });
      prevActiveRef.current = active;
    }
  }, [currentIndex, totalMatches, query, scrollContainerRef]);

  if (!isOpen) return null;

  const displayIndex = totalMatches > 0 ? currentIndex + 1 : 0;

  return (
    <div className="chat-search-toolbar" onKeyDown={handleKeyDown}>
      <div className="chat-search-toolbar__input-wrap">
        <Search size={14} className="chat-search-toolbar__icon" />
        <input
          ref={inputRef}
          className="chat-search-toolbar__input"
          type="text"
          value={rawQuery}
          onChange={handleInputChange}
          placeholder="Search messages..."
          spellCheck={false}
          autoComplete="off"
        />
      </div>
      <span className="chat-search-toolbar__count">
        {displayIndex}/{totalMatches}
      </span>
      <button
        type="button"
        className="chat-search-toolbar__nav-btn"
        onClick={goToPrev}
        disabled={totalMatches === 0}
        title="Previous match (Shift+Enter)"
        aria-label="Previous match"
      >
        <ChevronUp size={16} />
      </button>
      <button
        type="button"
        className="chat-search-toolbar__nav-btn"
        onClick={goToNext}
        disabled={totalMatches === 0}
        title="Next match (Enter)"
        aria-label="Next match"
      >
        <ChevronDown size={16} />
      </button>
      <button
        type="button"
        className="chat-search-toolbar__close-btn"
        onClick={close}
        title="Close search (Escape)"
        aria-label="Close search"
      >
        <X size={14} />
      </button>
    </div>
  );
}
