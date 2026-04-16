import { useState, useRef, useMemo, useCallback } from 'react';
import type { ComponentType } from 'react';
import { X, Search, ChevronDown } from 'lucide-react';
import { toc } from '@lobehub/icons';
import type { IconToc } from '@lobehub/icons';
import * as AllIcons from '@lobehub/icons';
import { Popover, PopoverTrigger, PopoverContent } from '../ui';

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

  // Focus search input and reset search when dropdown opens
  const handleOpenChange = useCallback((isOpen: boolean) => {
    setOpen(isOpen);
    if (isOpen) {
      setSearch('');
      requestAnimationFrame(() => searchRef.current?.focus());
    }
  }, []);

  // Scroll selected icon into view when dropdown opens
  const handleOpenAutoFocus = useCallback(() => {
    if (value && gridRef.current) {
      requestAnimationFrame(() => {
        const selected = gridRef.current?.querySelector('[data-selected="true"]');
        selected?.scrollIntoView({ block: 'nearest' });
      });
    }
  }, [value]);

  const handleSelect = useCallback(
    (iconId: string) => {
      onChange(iconId);
      setOpen(false);
    },
    [onChange],
  );

  const handleClear = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      onChange(null);
    },
    [onChange],
  );

  const selectedIcon = value
    ? toc.find((item) => item.id.toLowerCase() === value.toLowerCase())
    : null;

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={[
            'flex items-center gap-2 w-full',
            'px-2.5 py-1.5 min-h-[32px]',
            'bg-surface-secondary',
            'border border-solid border-border',
            'rounded-sm',
            'text-text-primary',
            'cursor-pointer text-12px font-sans',
            'transition-all duration-150',
            'hover:border-[rgba(255,255,255,0.12)]',
            'data-[state=open]:border-[rgba(255,255,255,0.15)]',
          ].join(' ')}
        >
          {selectedIcon ? (
            <>
              <span
                className={[
                  'inline-flex items-center justify-center',
                  'w-[22px] h-[22px] shrink-0',
                  'rounded-sm',
                  'bg-surface-tertiary',
                  'border border-solid border-border',
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
                  'rounded-sm shrink-0',
                  'text-text-muted',
                  'transition-all duration-150',
                  'hover:(text-error bg-error-subtle)',
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
                  'rounded-sm',
                  'bg-surface-tertiary',
                  'border border-solid border-border',
                  'text-text-muted',
                ].join(' ')}
              >
                <Search size={11} />
              </span>
              <span className="flex-1 text-left text-text-muted">Select icon...</span>
              <ChevronDown
                size={12}
                className={[
                  'shrink-0 text-text-muted op-70',
                  'transition-transform duration-200',
                  open ? 'rotate-180' : '',
                ].join(' ')}
              />
            </>
          )}
        </button>
      </PopoverTrigger>

      <PopoverContent
        className="w-[320px] p-0 overflow-hidden"
        align="start"
        sideOffset={4}
        onOpenAutoFocus={handleOpenAutoFocus}
      >
        {/* Search bar */}
        <div className="flex items-center gap-1.5 px-2.5 py-2 border-b border-solid border-border bg-surface-secondary">
          <Search size={13} className="shrink-0 text-text-muted" />
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
              'text-text-primary',
              'placeholder:text-text-muted',
            ].join(' ')}
          />
          {search && (
            <button
              type="button"
              className={[
                'flex items-center justify-center',
                'w-[16px] h-[16px] shrink-0',
                'bg-transparent border-none',
                'text-text-muted',
                'cursor-pointer rounded-sm',
                'transition-colors duration-100',
                'hover:text-text-primary',
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
              <Search size={20} className="text-text-muted op-40" />
              <span className="text-12px text-text-muted">
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
                      'rounded-md',
                      'cursor-pointer text-left',
                      'transition-all duration-100',
                      isSelected
                        ? 'border-accent bg-accent-subtle text-accent shadow-[0_0_0_1px_var(--accent-subtle)]'
                        : 'border-border bg-surface-tertiary text-text-secondary hover:(bg-surface-hover text-text-primary border-[rgba(255,255,255,0.12)])',
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
                      isSelected ? 'text-accent' : 'text-text-muted',
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
          'border-t border-solid border-border',
          'bg-surface-secondary',
          'text-[10px] text-text-muted',
        ].join(' ')}>
          <span>{filteredIcons.length} item{filteredIcons.length !== 1 ? 's' : ''}</span>
          {value && selectedIcon && (
            <span className="text-accent font-500">
              {selectedIcon.title}
            </span>
          )}
        </div>
      </PopoverContent>
    </Popover>
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
