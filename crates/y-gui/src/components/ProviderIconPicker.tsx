import { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import { X } from 'lucide-react';
import { toc, type IconToc } from '@lobehub/icons';
import './ProviderIconPicker.css';

// CDN URL for icon images -- uses the static PNG CDN for light mode, color variant.
function iconCdnUrl(id: string): string {
  return `https://registry.npmmirror.com/@lobehub/icons-static-png/latest/files/light/${id}-color.png`;
}

// Fallback: mono variant (no -color suffix).
function iconCdnUrlMono(id: string): string {
  return `https://registry.npmmirror.com/@lobehub/icons-static-png/latest/files/light/${id}.png`;
}

interface ProviderIconPickerProps {
  value: string | null;
  onChange: (iconId: string | null) => void;
}

/** Searchable dropdown for selecting a provider icon from @lobehub/icons. */
export function ProviderIconPicker({ value, onChange }: ProviderIconPickerProps) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const containerRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  // Filter the icon table of contents by search query.
  const filteredIcons: IconToc[] = useMemo(() => {
    const q = search.toLowerCase().trim();
    if (!q) return toc;
    return toc.filter(
      (item) =>
        item.id.toLowerCase().includes(q) ||
        item.title.toLowerCase().includes(q) ||
        item.fullTitle.toLowerCase().includes(q),
    );
  }, [search]);

  // Close dropdown on outside click.
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  // Focus search input when dropdown opens.
  useEffect(() => {
    if (open) {
      searchRef.current?.focus();
    }
  }, [open]);

  const handleSelect = useCallback(
    (iconId: string) => {
      onChange(iconId);
      setOpen(false);
      setSearch('');
    },
    [onChange],
  );

  const handleClear = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      onChange(null);
      setSearch('');
    },
    [onChange],
  );

  const selectedIcon = value ? toc.find((item) => item.id.toLowerCase() === value.toLowerCase()) : null;

  return (
    <div className="icon-picker" ref={containerRef}>
      <button
        type="button"
        className="icon-picker-trigger"
        onClick={() => setOpen(!open)}
      >
        {selectedIcon ? (
          <>
            <img
              className="icon-picker-preview"
              src={iconCdnUrl(selectedIcon.id.toLowerCase())}
              alt={selectedIcon.title}
              onError={(e) => {
                (e.target as HTMLImageElement).src = iconCdnUrlMono(selectedIcon.id.toLowerCase());
              }}
            />
            <span className="icon-picker-trigger-label">{selectedIcon.title}</span>
            <span
              className="icon-picker-clear"
              role="button"
              tabIndex={0}
              onClick={handleClear}
              onKeyDown={(e) => { if (e.key === 'Enter') handleClear(e as unknown as React.MouseEvent); }}
            >
              <X size={12} />
            </span>
          </>
        ) : (
          <span className="icon-picker-trigger-placeholder">Select icon...</span>
        )}
      </button>

      {open && (
        <div className="icon-picker-dropdown">
          <div className="icon-picker-search-wrap">
            <input
              ref={searchRef}
              className="icon-picker-search"
              type="text"
              placeholder="Search icons..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>
          <div className="icon-picker-list">
            {filteredIcons.length === 0 ? (
              <div className="icon-picker-empty">No icons found</div>
            ) : (
              filteredIcons.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className={`icon-picker-item ${value === item.id.toLowerCase() ? 'selected' : ''}`}
                  onClick={() => handleSelect(item.id.toLowerCase())}
                >
                  <img
                    className="icon-picker-item-img"
                    src={iconCdnUrl(item.id.toLowerCase())}
                    alt={item.title}
                    loading="lazy"
                    onError={(e) => {
                      (e.target as HTMLImageElement).src = iconCdnUrlMono(item.id.toLowerCase());
                    }}
                  />
                  <span className="icon-picker-item-label">{item.title}</span>
                </button>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}

/** Small inline icon for a provider, using CDN images. Falls back to null if no iconId. */
export function ProviderIconImg({
  iconId,
  size = 16,
  className = '',
}: {
  iconId: string | null | undefined;
  size?: number;
  className?: string;
}) {
  if (!iconId) return null;
  return (
    <img
      className={`provider-icon-img ${className}`}
      src={iconCdnUrl(iconId)}
      alt=""
      width={size}
      height={size}
      onError={(e) => {
        (e.target as HTMLImageElement).src = iconCdnUrlMono(iconId);
      }}
    />
  );
}
