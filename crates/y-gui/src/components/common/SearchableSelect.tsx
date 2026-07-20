import { useRef, useState, type ReactNode } from 'react';
import { Search } from 'lucide-react';

import { Popover, PopoverContent, PopoverTrigger } from '../ui';
import {
  filterSearchableOptions,
  type SearchableOption,
} from './searchableSelectUtils';
import './SearchableSelect.css';

interface SearchableOptionListProps {
  options: SearchableOption[];
  query: string;
  searchPlaceholder: string;
  emptyMessage: string;
  selectedValue?: string;
  onQueryChange: (query: string) => void;
  onSelect: (value: string) => void;
  renderOption?: (option: SearchableOption, selected: boolean) => ReactNode;
  inputRef?: React.RefObject<HTMLInputElement | null>;
}

export function SearchableOptionList({
  options,
  query,
  searchPlaceholder,
  emptyMessage,
  selectedValue,
  onQueryChange,
  onSelect,
  renderOption,
  inputRef,
}: SearchableOptionListProps) {
  const filteredOptions = filterSearchableOptions(options, query);

  return (
    <div className="searchable-select-panel">
      <div className="searchable-select-search">
        <Search size={13} aria-hidden="true" />
        <input
          ref={inputRef}
          type="search"
          value={query}
          placeholder={searchPlaceholder}
          aria-label={searchPlaceholder}
          onChange={(event) => onQueryChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && filteredOptions.length === 1) {
              event.preventDefault();
              onSelect(filteredOptions[0].value);
            }
          }}
        />
      </div>

      {filteredOptions.length === 0 ? (
        <div className="searchable-select-empty">{emptyMessage}</div>
      ) : (
        <div className="searchable-select-options" role="listbox">
          {filteredOptions.map((option) => {
            const selected = option.value === selectedValue;
            return (
              <button
                key={option.value}
                type="button"
                role="option"
                aria-selected={selected}
                className={`searchable-select-option${selected ? ' selected' : ''}`}
                onClick={() => onSelect(option.value)}
              >
                {renderOption ? renderOption(option, selected) : (
                  <>
                    <span className="searchable-select-option-label">{option.label}</span>
                    {option.description && (
                      <span className="searchable-select-option-description">
                        {option.description}
                      </span>
                    )}
                  </>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

interface SearchableSelectProps {
  options: SearchableOption[];
  value?: string;
  onValueChange: (value: string) => void;
  searchPlaceholder: string;
  emptyMessage: string;
  children: ReactNode;
  contentClassName?: string;
  side?: 'top' | 'right' | 'bottom' | 'left';
  align?: 'start' | 'center' | 'end';
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  renderOption?: (option: SearchableOption, selected: boolean) => ReactNode;
}

export function SearchableSelect({
  options,
  value,
  onValueChange,
  searchPlaceholder,
  emptyMessage,
  children,
  contentClassName,
  side = 'bottom',
  align = 'start',
  open,
  onOpenChange,
  renderOption,
}: SearchableSelectProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const [query, setQuery] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  const resolvedOpen = open ?? internalOpen;

  const setOpen = (nextOpen: boolean) => {
    if (open === undefined) setInternalOpen(nextOpen);
    onOpenChange?.(nextOpen);
    if (nextOpen) {
      setQuery('');
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  };

  return (
    <Popover open={resolvedOpen} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent
        side={side}
        align={align}
        className={['searchable-select-content', contentClassName].filter(Boolean).join(' ')}
      >
        <SearchableOptionList
          options={options}
          query={query}
          searchPlaceholder={searchPlaceholder}
          emptyMessage={emptyMessage}
          selectedValue={value}
          onQueryChange={setQuery}
          onSelect={(nextValue) => {
            onValueChange(nextValue);
            setOpen(false);
          }}
          renderOption={renderOption}
          inputRef={inputRef}
        />
      </PopoverContent>
    </Popover>
  );
}
