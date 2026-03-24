// ---------------------------------------------------------------------------
// TagChipInput -- interactive chip-based tag editor (reusable component)
// ---------------------------------------------------------------------------

import { useState, useRef } from 'react';
import { Badge, Input } from '../ui';

export function TagChipInput({
  tags,
  onChange,
}: {
  tags: string[];
  onChange: (next: string[]) => void;
}) {
  const [inputValue, setInputValue] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const addTag = (raw: string) => {
    const trimmed = raw.trim().replace(/,$/, '');
    if (trimmed && !tags.includes(trimmed)) {
      onChange([...tags, trimmed]);
    }
    setInputValue('');
  };

  const removeTag = (index: number) => {
    onChange(tags.filter((_, i) => i !== index));
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      addTag(inputValue);
    } else if (e.key === 'Backspace' && inputValue === '' && tags.length > 0) {
      onChange(tags.slice(0, -1));
    }
  };

  const handleBlur = () => {
    if (inputValue.trim()) {
      addTag(inputValue);
    }
  };

  return (
    <div
      className={[
        'flex flex-wrap items-center gap-1',
        'min-h-8 px-2 py-1',
        'border border-solid border-[var(--border)]',
        'rounded-[var(--radius-sm)]',
        'bg-[var(--surface-secondary)]',
        'cursor-text',
        'transition-colors duration-150',
        'focus-within:border-[rgba(255,255,255,0.15)]',
      ].join(' ')}
      onClick={() => inputRef.current?.focus()}
    >
      {tags.map((tag, i) => (
        <Badge key={i} variant="outline" onDismiss={() => removeTag(i)}>
          {tag}
        </Badge>
      ))}
      <input
        ref={inputRef}
        className={[
          'flex-1 min-w-20',
          'border-none bg-transparent',
          'text-[var(--text-primary)]',
          'text-12px font-sans',
          'outline-none',
          'py-0.5 px-0',
          'leading-[1.4]',
          'placeholder:text-[var(--text-muted)]',
        ].join(' ')}
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        onKeyDown={handleKeyDown}
        onBlur={handleBlur}
        placeholder={tags.length === 0 ? 'Add tags (Enter or comma to confirm)' : ''}
      />
    </div>
  );
}
