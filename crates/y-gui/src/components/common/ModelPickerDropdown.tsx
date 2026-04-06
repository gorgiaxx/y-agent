// ---------------------------------------------------------------------------
// ModelPickerDropdown -- shared filterable model list overlay
//
// Used by ProvidersTab and SetupWizard for model discovery/selection.
// ---------------------------------------------------------------------------

import { useState, useEffect, useRef } from 'react';
import { Search } from 'lucide-react';
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
  onClose: () => void;
  className?: string;
}

export function ModelPickerDropdown({
  models,
  loading,
  error,
  onSelect,
  onClose,
  className,
}: ModelPickerDropdownProps) {
  const [filter, setFilter] = useState('');
  const dropdownRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto-focus the filter input when dropdown opens.
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Close on click outside.
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [onClose]);

  const filtered = models.filter((m) =>
    m.id.toLowerCase().includes(filter.toLowerCase()) ||
    (m.display_name ?? '').toLowerCase().includes(filter.toLowerCase()),
  );

  return (
    <div
      className={['model-picker-dropdown', className].filter(Boolean).join(' ')}
      ref={dropdownRef}
    >
      <div className="model-picker-search">
        <Search size={12} className="model-picker-search-icon" />
        <input
          ref={inputRef}
          className="model-picker-filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter models..."
          onKeyDown={(e) => {
            if (e.key === 'Escape') onClose();
          }}
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
              onClick={() => { onSelect(m.id); onClose(); }}
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
    </div>
  );
}
