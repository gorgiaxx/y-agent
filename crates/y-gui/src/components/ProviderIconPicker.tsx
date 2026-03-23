import { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import type { ComponentType } from 'react';
import { X, Server } from 'lucide-react';
import { toc } from '@lobehub/icons';
import type { IconToc } from '@lobehub/icons';
import './ProviderIconPicker.css';

// ---------------------------------------------------------------------------
// Dynamic icon loader -- imports the React component from @lobehub/icons/es/<Id>
// ---------------------------------------------------------------------------

// Cache for loaded icon components to avoid re-importing.
const iconCache = new Map<string, ComponentType<{ size?: number | string }> | null>();

/** Dynamically import an icon component by its toc ID (PascalCase). */
async function loadIconComponent(tocId: string): Promise<ComponentType<{ size?: number | string }> | null> {
  if (iconCache.has(tocId)) return iconCache.get(tocId)!;
  try {
    // Each icon folder exports default = Mono (SVG) component.
    const mod = await import(/* @vite-ignore */ `@lobehub/icons/es/${tocId}`);
    const component = mod.default as ComponentType<{ size?: number | string }>;
    iconCache.set(tocId, component);
    return component;
  } catch {
    iconCache.set(tocId, null);
    return null;
  }
}

// ---------------------------------------------------------------------------
// DynamicIcon -- renders a single icon by its toc ID
// ---------------------------------------------------------------------------

function DynamicIcon({ tocId, size = 16 }: { tocId: string; size?: number }) {
  const [Icon, setIcon] = useState<ComponentType<{ size?: number | string }> | null>(
    iconCache.get(tocId) ?? null,
  );

  useEffect(() => {
    let cancelled = false;
    if (iconCache.has(tocId)) {
      setIcon(iconCache.get(tocId)!);
      return;
    }
    loadIconComponent(tocId).then((c) => {
      if (!cancelled) setIcon(() => c);
    });
    return () => { cancelled = true; };
  }, [tocId]);

  if (!Icon) return null;
  return <Icon size={size} />;
}

// ---------------------------------------------------------------------------
// ProviderIconPicker -- searchable dropdown
// ---------------------------------------------------------------------------

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

  const selectedIcon = value
    ? toc.find((item) => item.id.toLowerCase() === value.toLowerCase())
    : null;

  return (
    <div className="icon-picker" ref={containerRef}>
      <button
        type="button"
        className="icon-picker-trigger"
        onClick={() => setOpen(!open)}
      >
        {selectedIcon ? (
          <>
            <span className="icon-picker-preview">
              <DynamicIcon tocId={selectedIcon.id} size={18} />
            </span>
            <span className="icon-picker-trigger-label">{selectedIcon.fullTitle}</span>
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
                  className={`icon-picker-item ${value?.toLowerCase() === item.id.toLowerCase() ? 'selected' : ''}`}
                  onClick={() => handleSelect(item.id)}
                >
                  <span className="icon-picker-item-icon">
                    <DynamicIcon tocId={item.id} size={18} />
                  </span>
                  <span className="icon-picker-item-label">{item.fullTitle}</span>
                </button>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// ProviderIconImg -- inline icon for sidebar, status bar, dropdowns
// ---------------------------------------------------------------------------

/** Inline icon rendered from @lobehub/icons React components. Falls back to null. */
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

  // Resolve toc ID (PascalCase) from stored value (may be lowercase).
  const tocEntry = toc.find((item) => item.id.toLowerCase() === iconId.toLowerCase());
  if (!tocEntry) {
    // If not found in toc, try using the raw value as PascalCase id.
    return (
      <span className={`provider-icon-img ${className}`} style={{ display: 'inline-flex', width: size, height: size }}>
        <DynamicIcon tocId={iconId} size={size} />
      </span>
    );
  }

  return (
    <span className={`provider-icon-img ${className}`} style={{ display: 'inline-flex', width: size, height: size }}>
      <DynamicIcon tocId={tocEntry.id} size={size} />
    </span>
  );
}
