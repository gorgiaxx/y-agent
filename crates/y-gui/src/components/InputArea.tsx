import { useState, useRef, useCallback, useEffect } from 'react';
import { Square, X } from 'lucide-react';
import { ProviderSelector } from './ProviderSelector';
import { CommandMenu } from './CommandMenu';
import type { GuiCommandDef } from '../commands';
import type { ProviderInfo, SkillInfo } from '../types';
import type { PendingEdit } from '../hooks/useChat';
import './InputArea.css';

interface InputAreaProps {
  onSend: (message: string, skills?: string[]) => void;
  onStop?: () => void;
  onCommand?: (commandName: string) => boolean;
  disabled: boolean;
  sendOnEnter: boolean;
  providers: ProviderInfo[];
  selectedProviderId: string;
  onSelectProvider: (id: string) => void;
  pendingEdit?: PendingEdit | null;
  onCancelEdit?: () => void;
  skills?: SkillInfo[];
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
  skills = [],
}: InputAreaProps) {
  const [value, setValue] = useState('');
  const [commandMode, setCommandMode] = useState(false);
  const [selectedSkills, setSelectedSkills] = useState<string[]>([]);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const resetInput = useCallback(() => {
    setValue('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  }, []);

  const exitCommandMode = useCallback(() => {
    setCommandMode(false);
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
        textareaRef.current?.focus();
      }
    },
    [onCommand, resetInput, exitCommandMode],
  );

  const handleSkillSelect = useCallback(
    (skillName: string) => {
      exitCommandMode();
      resetInput();
      setSelectedSkills((prev) => {
        if (prev.includes(skillName)) return prev;
        return [...prev, skillName];
      });
      // Re-focus the textarea after skill selection.
      setTimeout(() => textareaRef.current?.focus(), 0);
    },
    [exitCommandMode, resetInput],
  );

  const handleRemoveSkill = useCallback((skillName: string) => {
    setSelectedSkills((prev) => prev.filter((s) => s !== skillName));
    textareaRef.current?.focus();
  }, []);

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

    onSend(trimmed, selectedSkills.length > 0 ? selectedSkills : undefined);
    resetInput();
    exitCommandMode();
    setSelectedSkills([]);
  }, [value, disabled, onSend, onCommand, resetInput, exitCommandMode, selectedSkills]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // In command mode, let the CommandMenu handle all keyboard events.
    if (commandMode) {
      // Only handle Escape here to also reset input.
      if (e.key === 'Escape') {
        e.preventDefault();
        exitCommandMode();
        resetInput();
        return;
      }
      // Let other keys propagate to CommandMenu's own onKeyDown.
      return;
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
            skills={skills}
            onSelect={handleCommandSelect}
            onSelectSkill={handleSkillSelect}
            onDismiss={() => {
              exitCommandMode();
              resetInput();
            }}
          />
        )}

        {/* Skill tags + textarea wrapper */}
        <div className="input-content">
          {selectedSkills.length > 0 && (
            <div className="skill-tags-row">
              {selectedSkills.map((name) => (
                <span key={name} className="skill-tag">
                  <span className="skill-tag-name">{name}</span>
                  <button
                    className="skill-tag-remove"
                    onClick={() => handleRemoveSkill(name)}
                    title={`Remove ${name}`}
                    aria-label={`Remove skill ${name}`}
                  >
                    <X size={10} />
                  </button>
                </span>
              ))}
            </div>
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
        </div>

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
