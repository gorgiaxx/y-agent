import { useState, useRef, useCallback, useEffect } from 'react';
import { Square, X } from 'lucide-react';
import { ProviderSelector } from './ProviderSelector';
import { CommandMenu } from './CommandMenu';
import { filterCommands } from '../commands';
import type { GuiCommandDef } from '../commands';
import type { ProviderInfo } from '../types';
import type { PendingEdit } from '../hooks/useChat';
import './InputArea.css';

interface InputAreaProps {
  onSend: (message: string) => void;
  onStop?: () => void;
  onCommand?: (commandName: string) => boolean;
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
  onCommand,
  disabled,
  sendOnEnter,
  providers,
  selectedProviderId,
  onSelectProvider,
  pendingEdit,
  onCancelEdit,
}: InputAreaProps) {
  const [value, setValue] = useState('');
  const [commandMode, setCommandMode] = useState(false);
  const [commandQuery, setCommandQuery] = useState('');
  const [selectedCommandIndex, setSelectedCommandIndex] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const resetInput = useCallback(() => {
    setValue('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  }, []);

  const exitCommandMode = useCallback(() => {
    setCommandMode(false);
    setCommandQuery('');
    setSelectedCommandIndex(0);
  }, []);

  const handleCommandSelect = useCallback(
    (cmd: GuiCommandDef) => {
      exitCommandMode();
      if (cmd.immediate) {
        resetInput();
        onCommand?.(cmd.name);
      } else {
        // For non-immediate commands, insert the command text for further argument editing.
        setValue(`/${cmd.name} `);
      }
    },
    [onCommand, resetInput, exitCommandMode],
  );

  const handleSend = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed || disabled) return;

    // Intercept slash commands.
    if (trimmed.startsWith('/')) {
      const parts = trimmed.slice(1).split(/\s+/);
      const cmdName = parts[0];
      if (cmdName && onCommand?.(cmdName)) {
        resetInput();
        exitCommandMode();
        return;
      }
    }

    onSend(trimmed);
    resetInput();
    exitCommandMode();
  }, [value, disabled, onSend, onCommand, resetInput, exitCommandMode]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Command mode keyboard navigation.
    if (commandMode) {
      const filtered = filterCommands(commandQuery);

      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedCommandIndex((i) => Math.max(0, i - 1));
        return;
      }
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedCommandIndex((i) => Math.min(filtered.length - 1, i + 1));
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        exitCommandMode();
        resetInput();
        return;
      }
      if (e.key === 'Tab') {
        e.preventDefault();
        if (filtered.length > 0) {
          const idx = Math.min(selectedCommandIndex, filtered.length - 1);
          handleCommandSelect(filtered[idx]);
        }
        return;
      }
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        if (filtered.length > 0) {
          const idx = Math.min(selectedCommandIndex, filtered.length - 1);
          handleCommandSelect(filtered[idx]);
        }
        return;
      }
    }

    // Normal mode: Enter to send.
    if (sendOnEnter && e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value;
    setValue(val);

    // Auto-resize textarea.
    const el = e.target;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;

    // Command mode detection: "/" at start, single-line only.
    if (val.startsWith('/') && !val.includes('\n')) {
      setCommandMode(true);
      setCommandQuery(val.slice(1));
      setSelectedCommandIndex(0);
    } else {
      if (commandMode) exitCommandMode();
    }
  };

  // When entering edit mode, populate the textarea with the message content.
  useEffect(() => {
    if (pendingEdit) {
      setValue(pendingEdit.content);
      exitCommandMode();
      if (textareaRef.current) {
        textareaRef.current.focus();
        // Auto-resize for the new content.
        textareaRef.current.style.height = 'auto';
        textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 200)}px`;
      }
    }
  }, [pendingEdit, exitCommandMode]);

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
        {commandMode && (
          <CommandMenu
            query={commandQuery}
            selectedIndex={selectedCommandIndex}
            onSelect={handleCommandSelect}
            onDismiss={() => {
              exitCommandMode();
              resetInput();
            }}
            onHover={setSelectedCommandIndex}
          />
        )}
        <textarea
          ref={textareaRef}
          className="input-textarea"
          value={value}
          onChange={handleInput}
          onKeyDown={handleKeyDown}
          placeholder={disabled ? 'Waiting for response...' : 'Type a message... (/ for commands)'}
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
          {commandMode
            ? 'Up/Down to navigate, Enter to select, Esc to dismiss'
            : sendOnEnter
              ? 'Enter to send, Shift+Enter for newline'
              : 'Shift+Enter to send'}
        </div>
      </div>
    </div>
  );
}
