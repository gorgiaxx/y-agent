import { useState, useRef, useCallback, useEffect } from 'react';
import { Square, X } from 'lucide-react';
import { ProviderSelector } from './ProviderSelector';
import type { ProviderInfo } from '../types';
import type { PendingEdit } from '../hooks/useChat';
import './InputArea.css';

interface InputAreaProps {
  onSend: (message: string) => void;
  onStop?: () => void;
  disabled: boolean;
  sendOnEnter: boolean;
  providers: ProviderInfo[];
  selectedProviderId: string;
  onSelectProvider: (id: string) => void;
  pendingEdit?: PendingEdit | null;
  onCancelEdit?: () => void;
}

export function InputArea({
  onSend,
  onStop,
  disabled,
  sendOnEnter,
  providers,
  selectedProviderId,
  onSelectProvider,
  pendingEdit,
  onCancelEdit,
}: InputAreaProps) {
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

  // When entering edit mode, populate the textarea with the message content.
  useEffect(() => {
    if (pendingEdit) {
      setValue(pendingEdit.content);
      if (textareaRef.current) {
        textareaRef.current.focus();
        // Auto-resize for the new content.
        textareaRef.current.style.height = 'auto';
        textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 200)}px`;
      }
    }
  }, [pendingEdit]);

  return (
    <div className="input-area">
      {pendingEdit && (
        <div className="edit-banner">
          <span className="edit-banner-text">Editing message -- sending will undo context to this point</span>
          <button
            className="edit-banner-dismiss"
            onClick={onCancelEdit}
            title="Cancel edit"
            aria-label="Cancel edit"
          >
            <X size={14} />
          </button>
        </div>
      )}
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
      <div className="input-footer">
        <ProviderSelector
          providers={providers}
          selectedProviderId={selectedProviderId}
          onSelect={onSelectProvider}
          disabled={disabled}
        />
        <div className="input-hint">
          {sendOnEnter ? 'Enter to send, Shift+Enter for newline' : 'Shift+Enter to send'}
        </div>
      </div>
    </div>
  );
}
