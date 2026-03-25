import { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import type { ComponentType } from 'react';
import { X, Search, ChevronDown } from 'lucide-react';
import { toc } from '@lobehub/icons';
import type { IconToc } from '@lobehub/icons';
import * as AllIcons from '@lobehub/icons';

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
// ProviderIconPicker -- searchable dropdown with icon grid
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
  const gridRef = useRef<HTMLDivElement>(null);

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

  // Scroll selected icon into view when dropdown opens
  useEffect(() => {
    if (open && value && gridRef.current) {
      requestAnimationFrame(() => {
        const selected = gridRef.current?.querySelector('[data-selected="true"]');
        selected?.scrollIntoView({ block: 'nearest' });
      });
    }
  }, [open, value]);

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
      {/* -- Trigger button -- */}
      <button
        type="button"
        className={[
          'flex items-center gap-2 w-full',
          'px-2.5 py-1.5 min-h-[32px]',
          'bg-[var(--surface-secondary)]',
          'border border-solid',
          open
            ? 'border-[rgba(255,255,255,0.15)]'
            : 'border-[var(--border)]',
          'rounded-[var(--radius-sm)]',
          'text-[var(--text-primary)]',
          'cursor-pointer text-12px font-sans',
          'transition-all duration-150',
          'hover:border-[rgba(255,255,255,0.12)]',
        ].join(' ')}
        onClick={() => setOpen(!open)}
      >
        {selectedIcon ? (
          <>
            <span
              className={[
                'inline-flex items-center justify-center',
                'w-[22px] h-[22px] shrink-0',
                'rounded-[var(--radius-sm)]',
                'bg-[var(--surface-tertiary)]',
                'border border-solid border-[var(--border)]',
              ].join(' ')}
            >
              <DynamicIcon tocId={selectedIcon.id} size={14} />
            </span>
            <span className="flex-1 text-left overflow-hidden text-ellipsis whitespace-nowrap">
              {selectedIcon.fullTitle}
            </span>
            <span
              className={[
                'flex items-center justify-center w-[18px] h-[18px]',
                'rounded-[var(--radius-sm)] shrink-0',
                'text-[var(--text-muted)]',
                'transition-all duration-150',
                'hover:(text-[var(--error)] bg-[var(--error-subtle)])',
              ].join(' ')}
              role="button"
              tabIndex={0}
              onClick={handleClear}
              onKeyDown={(e) => { if (e.key === 'Enter') handleClear(e as unknown as React.MouseEvent); }}
            >
              <X size={11} />
            </span>
          </>
        ) : (
          <>
            <span
              className={[
                'inline-flex items-center justify-center',
                'w-[22px] h-[22px] shrink-0',
                'rounded-[var(--radius-sm)]',
                'bg-[var(--surface-tertiary)]',
                'border border-solid border-[var(--border)]',
                'text-[var(--text-muted)]',
              ].join(' ')}
            >
              <Search size={11} />
            </span>
            <span className="flex-1 text-left text-[var(--text-muted)]">Select icon...</span>
            <ChevronDown
              size={12}
              className={[
                'shrink-0 text-[var(--text-muted)] op-70',
                'transition-transform duration-200',
                open ? 'rotate-180' : '',
              ].join(' ')}
            />
          </>
        )}
      </button>

      {/* -- Dropdown panel -- */}
      {open && (
        <div
          className={[
            'absolute top-[calc(100%+4px)] left-0 z-100',
            'w-[320px]',
            'bg-[var(--surface-secondary)]',
            'border border-solid border-[var(--border)]',
            'rounded-[var(--radius-md)]',
            'shadow-[0_8px_32px_rgba(0,0,0,0.45)]',
            'overflow-hidden',
            'animate-[selectIn_0.12s_ease-out]',
          ].join(' ')}
        >
          {/* Search bar */}
          <div className="flex items-center gap-1.5 px-2.5 py-2 border-b border-solid border-[var(--surface-secondary)] bg-[var(--surface-secondary)]">
            <Search size={13} className="shrink-0 text-[var(--text-muted)]" />
            <input
              ref={searchRef}
              type="text"
              placeholder="Search..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className={[
                'flex-1 min-w-0',
                'bg-transparent border-none outline-none',
                'text-12px font-sans',
                'text-[var(--text-primary)]',
                'placeholder:text-[var(--text-muted)]',
              ].join(' ')}
            />
            {search && (
              <button
                type="button"
                className={[
                  'flex items-center justify-center',
                  'w-[16px] h-[16px] shrink-0',
                  'bg-transparent border-none',
                  'text-[var(--text-muted)]',
                  'cursor-pointer rounded-[var(--radius-sm)]',
                  'transition-colors duration-100',
                  'hover:text-[var(--text-primary)]',
                ].join(' ')}
                onClick={() => {
                  setSearch('');
                  searchRef.current?.focus();
                }}
              >
                <X size={11} />
              </button>
            )}
          </div>

          {/* Icon grid / empty state */}
          <div
            ref={gridRef}
            className="max-h-[280px] overflow-y-auto p-1.5"
          >
            {filteredIcons.length === 0 ? (
              <div className={[
                'flex flex-col items-center justify-center gap-1',
                'py-6 text-center',
              ].join(' ')}>
                <Search size={20} className="text-[var(--text-muted)] op-40" />
                <span className="text-12px text-[var(--text-muted)]">
                  No matching icons
                </span>
              </div>
            ) : (
              <div className="grid grid-cols-4 gap-1">
                {filteredIcons.map((item) => {
                  const isSelected = value?.toLowerCase() === item.id.toLowerCase();
                  return (
                    <button
                      key={item.id}
                      type="button"
                      data-selected={isSelected}
                      className={[
                        'flex flex-col items-center justify-center gap-1',
                        'py-2 px-1',
                        'border border-solid',
                        'rounded-[var(--radius-md)]',
                        'cursor-pointer text-left',
                        'transition-all duration-100',
                        isSelected
                          ? [
                              'border-[var(--accent)]',
                              'bg-[var(--accent-subtle)]',
                              'text-[var(--accent)]',
                              'shadow-[0_0_0_1px_var(--accent-subtle)]',
                            ].join(' ')
                          : [
                              'border-[var(--border)]',
                              'bg-[var(--surface-tertiary)]',
                              'text-[var(--text-secondary)]',
                              'hover:(bg-[var(--surface-hover)] text-[var(--text-primary)] border-[rgba(255,255,255,0.12)])',
                            ].join(' '),
                      ].join(' ')}
                      title={item.fullTitle}
                      onClick={() => handleSelect(item.id)}
                    >
                      <span className="inline-flex items-center justify-center w-6 h-6 shrink-0">
                        <DynamicIcon tocId={item.id} size={20} />
                      </span>
                      <span className={[
                        'text-[10px] leading-tight text-center',
                        'w-full overflow-hidden text-ellipsis whitespace-nowrap',
                        'font-sans',
                        isSelected ? 'text-[var(--accent)]' : 'text-[var(--text-muted)]',
                      ].join(' ')}>
                        {item.title}
                      </span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          {/* Footer count */}
          <div className={[
            'flex items-center justify-between',
            'px-2.5 py-1.5',
            'border-t border-solid border-[var(--surface-secondary)]',
            'bg-[var(--surface-secondary)]',
            'text-[10px] text-[var(--text-muted)]',
          ].join(' ')}>
            <span>{filteredIcons.length} item{filteredIcons.length !== 1 ? 's' : ''}</span>
            {value && selectedIcon && (
              <span className="text-[var(--accent)] font-500">
                {selectedIcon.title}
              </span>
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
