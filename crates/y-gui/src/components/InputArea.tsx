import { useState, useRef, useCallback } from 'react';
import { Square } from 'lucide-react';
import './InputArea.css';

interface InputAreaProps {
  onSend: (message: string) => void;
  onStop?: () => void;
  disabled: boolean;
  sendOnEnter: boolean;
}

export function InputArea({ onSend, onStop, disabled, sendOnEnter }: InputAreaProps) {
  const [value, setValue] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleSend = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed || disabled) return;
    onSend(trimmed);
    setValue('');
    // Reset textarea height.
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  }, [value, disabled, onSend]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (sendOnEnter && e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setValue(e.target.value);
    // Auto-resize textarea.
    const el = e.target;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  };

  return (
    <div className="input-area">
      <div className="input-container">
        <textarea
          ref={textareaRef}
          className="input-textarea"
          value={value}
          onChange={handleInput}
          onKeyDown={handleKeyDown}
          placeholder={disabled ? 'Waiting for response...' : 'Type a message...'}
          disabled={disabled}
          rows={1}
        />
        {disabled && onStop && (
          <button
            className="btn-stop"
            onClick={onStop}
            title="Stop generation"
            id="btn-stop-generation"
          >
            <Square size={14} />
          </button>
        )}
      </div>
      <div className="input-hint">
        {sendOnEnter ? 'Enter to send, Shift+Enter for newline' : 'Shift+Enter to send'}
      </div>
    </div>
  );
}
