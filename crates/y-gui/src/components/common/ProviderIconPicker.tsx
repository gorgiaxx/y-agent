import { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import type { ComponentType } from 'react';
import { X } from 'lucide-react';
import { toc } from '@lobehub/icons';
import type { IconToc } from '@lobehub/icons';
import * as AllIcons from '@lobehub/icons';
import { Input } from '../ui';

// ---------------------------------------------------------------------------
// Static icon lookup map -- all icons are bundled at build time
// ---------------------------------------------------------------------------

const allIconsRecord = AllIcons as Record<string, unknown>;
const iconMap: Record<string, ComponentType<{ size?: number | string }>> = {};

for (const entry of toc) {
  const comp = allIconsRecord[entry.id];
  if (comp) {
    iconMap[entry.id] = comp as ComponentType<{ size?: number | string }>;
  }
}

// ---------------------------------------------------------------------------
// DynamicIcon -- renders a single icon by its toc ID (synchronous)
// ---------------------------------------------------------------------------

function DynamicIcon({ tocId, size = 16 }: { tocId: string; size?: number }) {
  const Icon = iconMap[tocId];
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

  // Close dropdown on outside click
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

  // Focus search input when dropdown opens
  useEffect(() => {
    if (open) searchRef.current?.focus();
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
    <div className="relative" ref={containerRef}>
      <button
        type="button"
        className={[
          'flex items-center gap-1.5 w-full',
          'px-2 py-1.5 min-h-[30px]',
          'bg-[var(--surface-secondary)]',
          'border border-solid border-[var(--border)]',
          'rounded-[var(--radius-sm)]',
          'text-[var(--text-primary)]',
          'cursor-pointer text-12px font-sans',
          'transition-colors duration-150',
          'hover:border-[rgba(255,255,255,0.15)]',
        ].join(' ')}
        onClick={() => setOpen(!open)}
      >
        {selectedIcon ? (
          <>
            <span className="inline-flex items-center justify-center w-[18px] h-[18px] shrink-0">
              <DynamicIcon tocId={selectedIcon.id} size={18} />
            </span>
            <span className="flex-1 text-left overflow-hidden text-ellipsis whitespace-nowrap">
              {selectedIcon.fullTitle}
            </span>
            <span
              className={[
                'flex items-center justify-center w-[18px] h-[18px]',
                'rounded-full cursor-pointer',
                'text-[var(--text-muted)]',
                'transition-colors duration-150',
                'hover:text-[var(--text-primary)]',
              ].join(' ')}
              role="button"
              tabIndex={0}
              onClick={handleClear}
              onKeyDown={(e) => { if (e.key === 'Enter') handleClear(e as unknown as React.MouseEvent); }}
            >
              <X size={12} />
            </span>
          </>
        ) : (
          <span className="flex-1 text-left text-[var(--text-muted)]">Select icon...</span>
        )}
      </button>

      {open && (
        <div
          className={[
            'absolute top-[calc(100%+4px)] left-0 right-0 z-100',
            'bg-[var(--surface-primary)]',
            'border border-solid border-[var(--border)]',
            'rounded-[var(--radius-md)]',
            'shadow-md overflow-hidden',
            'animate-[selectIn_0.1s_ease-out]',
          ].join(' ')}
        >
          <div className="p-1.5 border-b border-solid border-[var(--border)]">
            <Input
              ref={searchRef}
              type="text"
              placeholder="Search icons..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>
          <div className="max-h-60 overflow-y-auto p-1">
            {filteredIcons.length === 0 ? (
              <div className="p-4 text-center text-[var(--text-muted)] text-12px">
                No icons found
              </div>
            ) : (
              filteredIcons.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className={[
                    'flex items-center gap-2 w-full',
                    'px-2 py-1.5',
                    'bg-none border-none',
                    'rounded-[var(--radius-sm)]',
                    'text-[var(--text-secondary)]',
                    'cursor-pointer text-12px font-sans text-left',
                    'transition-colors duration-100',
                    'hover:(bg-[var(--surface-hover)] text-[var(--text-primary)])',
                    value?.toLowerCase() === item.id.toLowerCase()
                      ? 'text-[var(--accent)] bg-[var(--accent-subtle)]'
                      : '',
                  ].join(' ')}
                  onClick={() => handleSelect(item.id)}
                >
                  <span className="inline-flex items-center justify-center w-5 h-5 shrink-0">
                    <DynamicIcon tocId={item.id} size={18} />
                  </span>
                  <span className="overflow-hidden text-ellipsis whitespace-nowrap">
                    {item.fullTitle}
                  </span>
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

  const tocEntry = toc.find((item) => item.id.toLowerCase() === iconId.toLowerCase());
  const resolvedId = tocEntry ? tocEntry.id : iconId;

  return (
    <span
      className={[
        'inline-flex items-center justify-center shrink-0 align-middle leading-none',
        className,
      ].join(' ')}
      style={{ width: size, height: size }}
    >
      <DynamicIcon tocId={resolvedId} size={size} />
    </span>
  );
}
