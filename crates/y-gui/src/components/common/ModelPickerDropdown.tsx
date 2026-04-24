// ---------------------------------------------------------------------------
// ModelPickerDropdown -- shared filterable model list overlay
//
// Used by ProvidersTab and SetupWizard for model discovery/selection.
// Wraps content in a Radix Popover for accessibility (escape, focus trap, aria).
// ---------------------------------------------------------------------------

import { useState, useRef } from 'react';
import { Search } from 'lucide-react';
import { Popover, PopoverTrigger, PopoverContent } from '../ui';
import './ModelPickerDropdown.css';

export interface ModelItem {
  id: string;
  display_name?: string;
}

interface ModelPickerDropdownProps {
  models: ModelItem[];
  loading: boolean;
  error: string | null;
  onSelect: (id: string) => void;
  /** Trigger element that opens the dropdown. */
  children: React.ReactNode;
  className?: string;
}

export function ModelPickerDropdown({
  models,
  loading,
  error,
  onSelect,
  children,
  className,
}: ModelPickerDropdownProps) {
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const filtered = models.filter((m) =>
    m.id.toLowerCase().includes(filter.toLowerCase()) ||
    (m.display_name ?? '').toLowerCase().includes(filter.toLowerCase()),
  );

  return (
    <Popover
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (nextOpen) {
          setFilter('');
          requestAnimationFrame(() => inputRef.current?.focus());
        }
      }}
    >
      <PopoverTrigger asChild>
        {children}
      </PopoverTrigger>
      <PopoverContent
        className={['model-picker-dropdown', className].filter(Boolean).join(' ')}
        align="start"
        sideOffset={4}
      >
        <div className="model-picker-search">
          <Search size={12} className="model-picker-search-icon" />
          <input
            ref={inputRef}
            className="model-picker-filter"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter models..."
          />
        </div>
        {loading && (
          <div className="model-picker-status">
            <span className="model-picker-spinner" /> Fetching models...
          </div>
        )}
        {error && (
          <div className="model-picker-status model-picker-error">{error}</div>
        )}
        {!loading && !error && filtered.length === 0 && (
          <div className="model-picker-status">No models found</div>
        )}
        {!loading && !error && filtered.length > 0 && (
          <div className="model-picker-list">
            {filtered.map((m) => (
              <button
                key={m.id}
                className="model-picker-item"
                onClick={() => { onSelect(m.id); setOpen(false); }}
                type="button"
              >
                <span className="model-picker-item-id">{m.id}</span>
                {m.display_name && m.display_name !== m.id && (
                  <span className="model-picker-item-name">{m.display_name}</span>
                )}
              </button>
            ))}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
