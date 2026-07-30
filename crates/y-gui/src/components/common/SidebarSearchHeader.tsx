import type { ReactNode, RefObject } from 'react';
import { Search } from 'lucide-react';

import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';

interface SidebarSearchHeaderProps {
  label: string;
  count: number;
  searchTitle: string;
  searchPlaceholder: string;
  searchOpen: boolean;
  searchQuery: string;
  searchInputRef: RefObject<HTMLInputElement | null>;
  onSearchQueryChange: (query: string) => void;
  onSearchToggle: () => void;
  actions?: ReactNode;
}

export function SidebarSearchHeader({
  label,
  count,
  searchTitle,
  searchPlaceholder,
  searchOpen,
  searchQuery,
  searchInputRef,
  onSearchQueryChange,
  onSearchToggle,
  actions,
}: SidebarSearchHeaderProps) {
  return (
    <>
      <div className="agent-session-toolbar">
        <div className="agent-session-toolbar-label">
          <span>{label}</span>
          <div className="agent-session-toolbar-meta">
            <Badge variant="outline">{count}</Badge>
          </div>
        </div>
        <div className="agent-session-toolbar-actions">
          <Button
            variant="icon"
            size="sm"
            onClick={onSearchToggle}
            title={searchTitle}
            aria-label={searchTitle}
            aria-expanded={searchOpen}
          >
            <Search size={14} />
          </Button>
          {actions}
        </div>
      </div>
      {searchOpen && (
        <div className="sidebar-inline-search">
          <input
            ref={searchInputRef}
            type="text"
            placeholder={searchPlaceholder}
            value={searchQuery}
            onChange={(event) => onSearchQueryChange(event.target.value)}
            className="sidebar-inline-search-input"
            onKeyDown={(event) => {
              if (event.key === 'Escape') onSearchToggle();
            }}
          />
        </div>
      )}
    </>
  );
}
