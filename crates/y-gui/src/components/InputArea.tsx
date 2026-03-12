import { useState, useRef, useCallback } from 'react';
import './InputArea.css';

interface InputAreaProps {
  onSend: (message: string) => void;
  disabled: boolean;
  sendOnEnter: boolean;
}

export function InputArea({ onSend, disabled, sendOnEnter }: InputAreaProps) {
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
        <button
          className="btn-send"
          onClick={handleSend}
          disabled={disabled || !value.trim()}
          title="Send message"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <line x1="22" y1="2" x2="11" y2="13" />
            <polygon points="22 2 15 22 11 13 2 9 22 2" />
          </svg>
        </button>
      </div>
      <div className="input-hint">
        {sendOnEnter ? 'Enter to send, Shift+Enter for newline' : 'Shift+Enter to send'}
      </div>
    </div>
  );
}
