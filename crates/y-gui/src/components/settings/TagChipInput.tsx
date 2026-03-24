// ---------------------------------------------------------------------------
// TagChipInput -- interactive chip-based tag editor (reusable component)
// ---------------------------------------------------------------------------

import { useState, useRef } from 'react';

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
    <div className="pf-tag-input-wrap" onClick={() => inputRef.current?.focus()}>
      {tags.map((tag, i) => (
        <span key={i} className="pf-tag-chip">
          <span className="pf-tag-chip-text">{tag}</span>
          <button
            type="button"
            className="pf-tag-chip-remove"
            onClick={(e) => { e.stopPropagation(); removeTag(i); }}
            title={`Remove tag "${tag}"`}
          >
            x
          </button>
        </span>
      ))}
      <input
        ref={inputRef}
        className="pf-tag-text-input"
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        onKeyDown={handleKeyDown}
        onBlur={handleBlur}
        placeholder={tags.length === 0 ? 'Add tags (Enter or comma to confirm)' : ''}
      />
    </div>
  );
}
