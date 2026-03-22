import { useState, useRef, useCallback, useEffect } from 'react';
import { Square, X, AtSign, Maximize2, Minimize2, Paintbrush, Eraser, BookOpen } from 'lucide-react';
import { ConfirmDialog } from './ConfirmDialog';
import { CommandMenu } from './CommandMenu';
import type { GuiCommandDef } from '../commands';
import type { ProviderInfo, SkillInfo, KnowledgeCollectionInfo } from '../types';
import type { PendingEdit } from '../hooks/useChat';
import './InputArea.css';

interface InputAreaProps {
  onSend: (message: string, skills?: string[], knowledgeCollections?: string[]) => void;
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
  knowledgeCollections?: KnowledgeCollectionInfo[];
  expanded?: boolean;
  onExpandChange?: (expanded: boolean) => void;
  onClearSession?: () => void;
  onAddContextReset?: () => void;
}

/** Data attribute used to identify skill mention tokens in the contenteditable. */
const SKILL_ATTR = 'data-skill-name';


/**
 * Extract plain text and skill names from the contenteditable div.
 * Skill mentions are embedded as <span data-skill-name="..."> elements.
 * Recursively traverses all child nodes since browsers may wrap content
 * in <div> elements.
 */
function extractContent(el: HTMLDivElement): { text: string; skills: string[] } {
  const skills: string[] = [];
  let text = '';

  function walk(node: Node) {
    if (node.nodeType === Node.TEXT_NODE) {
      text += node.textContent || '';
    } else if (node.nodeType === Node.ELEMENT_NODE) {
      const element = node as HTMLElement;
      const skillName = element.getAttribute(SKILL_ATTR);
      if (skillName) {
        if (!skills.includes(skillName)) {
          skills.push(skillName);
        }
        return; // Don't descend into skill mention spans.
      } else if (element.tagName === 'BR') {
        text += '\n';
      } else {
        // Recurse into child elements (e.g. browser-inserted <div> wrappers).
        if (element.tagName === 'DIV' && element !== el) {
          // Browser wraps new lines in <div> — treat as newline.
          if (text.length > 0 && !text.endsWith('\n')) {
            text += '\n';
          }
        }
        for (const child of Array.from(element.childNodes)) {
          walk(child);
        }
      }
    }
  }

  for (const child of Array.from(el.childNodes)) {
    walk(child);
  }

  console.debug('[InputArea] extractContent:', { text: text.trim(), skills });
  return { text, skills };
}

/** Get the plain text content (without skill tags) for command detection. */
function getPlainText(el: HTMLDivElement): string {
  let text = '';

  function walk(node: Node) {
    if (node.nodeType === Node.TEXT_NODE) {
      text += node.textContent || '';
    } else if (node.nodeType === Node.ELEMENT_NODE) {
      const element = node as HTMLElement;
      if (element.getAttribute(SKILL_ATTR)) {
        // Skip mention tokens — they're not part of the text.
      } else if (element.tagName === 'BR') {
        text += '\n';
      } else {
        if (element.tagName === 'DIV' && element !== el) {
          if (text.length > 0 && !text.endsWith('\n')) {
            text += '\n';
          }
        }
        for (const child of Array.from(element.childNodes)) {
          walk(child);
        }
      }
    }
  }

  for (const child of Array.from(el.childNodes)) {
    walk(child);
  }
  return text;
}

/** Create a skill mention DOM element. */
function createSkillMention(skillName: string): HTMLSpanElement {
  const span = document.createElement('span');
  span.setAttribute(SKILL_ATTR, skillName);
  span.setAttribute('contenteditable', 'false');
  span.className = 'skill-mention';
  span.textContent = `@${skillName}`;
  return span;
}



/** Place the cursor at the end of a contenteditable element. */
function placeCursorAtEnd(el: HTMLElement) {
  const range = document.createRange();
  const sel = window.getSelection();
  range.selectNodeContents(el);
  range.collapse(false);
  sel?.removeAllRanges();
  sel?.addRange(range);
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
  knowledgeCollections = [],
  expanded = false,
  onExpandChange,
  onClearSession,
  onAddContextReset,
}: InputAreaProps) {
  const [commandMode, setCommandMode] = useState(false);
  const [providerDropdownOpen, setProviderDropdownOpen] = useState(false);
  const [kbPickerOpen, setKbPickerOpen] = useState(false);
  const [selectedKbCollections, setSelectedKbCollections] = useState<string[]>([]);
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  const editableRef = useRef<HTMLDivElement>(null);
  const providerDropdownRef = useRef<HTMLDivElement>(null);
  const kbPickerRef = useRef<HTMLDivElement>(null);

  // Close provider dropdown on outside click.
  useEffect(() => {
    if (!providerDropdownOpen) return;
    const handleOutsideClick = (e: MouseEvent) => {
      if (providerDropdownRef.current && !providerDropdownRef.current.contains(e.target as Node)) {
        setProviderDropdownOpen(false);
      }
    };
    document.addEventListener('mousedown', handleOutsideClick);
    return () => document.removeEventListener('mousedown', handleOutsideClick);
  }, [providerDropdownOpen]);

  // Close knowledge picker on outside click.
  useEffect(() => {
    if (!kbPickerOpen) return;
    const handleOutsideClick = (e: MouseEvent) => {
      if (kbPickerRef.current && !kbPickerRef.current.contains(e.target as Node)) {
        setKbPickerOpen(false);
      }
    };
    document.addEventListener('mousedown', handleOutsideClick);
    return () => document.removeEventListener('mousedown', handleOutsideClick);
  }, [kbPickerOpen]);

  // Derive display label for selected provider.
  const selectedProviderLabel = selectedProviderId === 'auto'
    ? 'Auto'
    : providers.find((p) => p.id === selectedProviderId)?.model || selectedProviderId;

  const updateHasContent = useCallback(() => {
    if (!editableRef.current) return;
    // no-op currently — reserved for future use (e.g. enabling/disabling send button).
  }, []);

  const resetInput = useCallback(() => {
    if (editableRef.current) {
      editableRef.current.innerHTML = '';
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
        // For non-immediate commands, insert the command text.
        if (editableRef.current) {
          editableRef.current.textContent = `/${cmd.name} `;
          placeCursorAtEnd(editableRef.current);
          updateHasContent();
        }
      }
    },
    [onCommand, resetInput, exitCommandMode, updateHasContent],
  );

  const handleSkillSelect = useCallback(
    (skillName: string) => {
      exitCommandMode();
      if (!editableRef.current) return;

      // Check if skill is already mentioned.
      const existing = editableRef.current.querySelector(`[${SKILL_ATTR}="${skillName}"]`);
      if (existing) {
        // Already present — just clear the slash text and refocus.
        // Remove any "/" text that was typed for command mode.
        const textNodes = Array.from(editableRef.current.childNodes).filter(
          (n) => n.nodeType === Node.TEXT_NODE,
        );
        for (const tn of textNodes) {
          const t = tn.textContent || '';
          if (t.startsWith('/')) {
            tn.textContent = t.replace(/^\/\S*\s?/, '');
          }
        }
        placeCursorAtEnd(editableRef.current);
        updateHasContent();
        return;
      }

      // Remove the slash command text that triggered command mode.
      const textNodes = Array.from(editableRef.current.childNodes).filter(
        (n) => n.nodeType === Node.TEXT_NODE,
      );
      for (const tn of textNodes) {
        const t = tn.textContent || '';
        if (t.startsWith('/')) {
          tn.textContent = t.replace(/^\/\S*\s?/, '');
        }
      }

      // Insert the skill mention token.
      const mention = createSkillMention(skillName);
      editableRef.current.appendChild(mention);

      // Add a trailing space so the cursor has somewhere to go.
      const space = document.createTextNode('\u00A0');
      editableRef.current.appendChild(space);

      placeCursorAtEnd(editableRef.current);
      editableRef.current.focus();
      updateHasContent();
    },
    [exitCommandMode, updateHasContent],
  );

  const toggleKbCollection = useCallback((name: string) => {
    setSelectedKbCollections(prev =>
      prev.includes(name) ? prev.filter(n => n !== name) : [...prev, name]
    );
  }, []);

  const handleKbCollectionSelect = useCallback(
    (collectionName: string) => {
      exitCommandMode();
      if (!editableRef.current) return;

      // Remove the slash command text that triggered command mode.
      const textNodes = Array.from(editableRef.current.childNodes).filter(
        (n) => n.nodeType === Node.TEXT_NODE,
      );
      for (const tn of textNodes) {
        const t = tn.textContent || '';
        if (t.startsWith('/')) {
          tn.textContent = t.replace(/^\/\S*\s?/, '');
        }
      }

      // Use the same toggle logic as the toolbar KB button.
      toggleKbCollection(collectionName);

      placeCursorAtEnd(editableRef.current);
      editableRef.current.focus();
    },
    [exitCommandMode, toggleKbCollection],
  );

  const clearKbSelections = useCallback(() => {
    setSelectedKbCollections([]);
  }, []);

  const handleSend = useCallback(() => {
    if (!editableRef.current || disabled) return;

    const { text, skills: extractedSkills } = extractContent(editableRef.current);
    const trimmed = text.trim();

    if (!trimmed && extractedSkills.length === 0 && selectedKbCollections.length === 0) return;

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

    console.debug('[InputArea] handleSend:', { trimmed, extractedSkills, selectedKbCollections });
    onSend(
      trimmed,
      extractedSkills.length > 0 ? extractedSkills : undefined,
      selectedKbCollections.length > 0 ? selectedKbCollections : undefined,
    );
    resetInput();
    exitCommandMode();
  }, [disabled, onSend, onCommand, resetInput, exitCommandMode, selectedKbCollections]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Ignore key events during IME composition (e.g. Chinese pinyin input).
    // The Enter key used to confirm an IME composition should NOT trigger a
    // message send.  On some platforms a single Enter press during composition
    // can fire two keydown events (isComposing=true then isComposing=false),
    // which would cause a double-send without this guard.
    if (e.nativeEvent.isComposing || e.keyCode === 229) {
      return;
    }

    // In command mode, let the CommandMenu handle most keyboard events.
    if (commandMode) {
      if (e.key === 'Escape') {
        e.preventDefault();
        exitCommandMode();
        resetInput();
        return;
      }
      return;
    }

    // Normal mode: Enter to send.
    if (sendOnEnter && e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleInput = () => {
    if (!editableRef.current) return;

    const plainText = getPlainText(editableRef.current);
    updateHasContent();

    // Command mode detection: "/" at start, single-line only.
    if (plainText.startsWith('/') && !plainText.includes('\n')) {
      setCommandMode(true);
    } else {
      if (commandMode) exitCommandMode();
    }
  };

  // When entering edit mode, populate with the message content.
  useEffect(() => {
    if (pendingEdit && editableRef.current) {
      editableRef.current.textContent = pendingEdit.content;
      exitCommandMode();
      editableRef.current.focus();
      placeCursorAtEnd(editableRef.current);
      updateHasContent();
    }
  }, [pendingEdit, exitCommandMode, updateHasContent]);

  return (
    <div className={`input-area ${expanded ? 'input-area--expanded' : ''}`}>
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
            knowledgeCollections={knowledgeCollections}
            onSelect={handleCommandSelect}
            onSelectSkill={handleSkillSelect}
            onSelectKbCollection={handleKbCollectionSelect}
            onDismiss={() => {
              exitCommandMode();
              resetInput();
            }}
          />
        )}

        {/* Editable div with inline skill mentions */}
        <div className="input-content">
          <div
            ref={editableRef}
            className="input-editable"
            contentEditable={!disabled}
            onInput={handleInput}
            onKeyDown={handleKeyDown}
            data-placeholder={disabled ? 'Waiting for response...' : 'Type a message... (/ for commands), Enter to send, Shift+Enter for newline)'}
            role="textbox"
            aria-multiline="true"
            suppressContentEditableWarning
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

        {/* Toolbar row with action buttons — inside the input border */}
        <div className="input-toolbar">
          {/* (a) Model / provider selection */}
          <div className="toolbar-btn-group" ref={providerDropdownRef}>
            <button
              className="toolbar-btn has-tooltip"
              onClick={() => setProviderDropdownOpen(!providerDropdownOpen)}
              data-tooltip="Select model"
              disabled={disabled}
            >
              <AtSign size={14} />
              <span className="toolbar-btn-label">{selectedProviderLabel}</span>
            </button>
            {providerDropdownOpen && (
              <div className="toolbar-provider-dropdown">
                <button
                  className={`toolbar-provider-item ${selectedProviderId === 'auto' ? 'selected' : ''}`}
                  onClick={() => { onSelectProvider('auto'); setProviderDropdownOpen(false); }}
                >
                  Auto
                </button>
                {providers.map((p) => (
                  <button
                    key={p.id}
                    className={`toolbar-provider-item ${selectedProviderId === p.id ? 'selected' : ''}`}
                    onClick={() => { onSelectProvider(p.id); setProviderDropdownOpen(false); }}
                  >
                    {p.id} ({p.model})
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* (b) Expand / collapse input */}
          <button
            className="toolbar-btn has-tooltip"
            onClick={() => onExpandChange?.(!expanded)}
            data-tooltip={expanded ? 'Collapse input' : 'Expand input'}
          >
            {expanded ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
          </button>

          {/* (c) Clear all messages */}
          <button
            className="toolbar-btn toolbar-btn--danger has-tooltip"
            onClick={() => setClearConfirmOpen(true)}
            data-tooltip="Clear chat"
            disabled={disabled}
          >
            <Paintbrush size={14} />
          </button>

          {/* (d) Add context reset */}
          <button
            className="toolbar-btn has-tooltip"
            onClick={onAddContextReset}
            data-tooltip="Reset context"
            disabled={disabled}
          >
            <Eraser size={14} />
          </button>

          {/* (e) Knowledge base picker */}
          {knowledgeCollections.length > 0 && (
            <div className="toolbar-btn-group" ref={kbPickerRef}>
              <button
                className={`toolbar-btn has-tooltip ${selectedKbCollections.length > 0 ? 'toolbar-btn--active' : ''}`}
                onClick={() => setKbPickerOpen(!kbPickerOpen)}
                data-tooltip="Knowledge bases"
                disabled={disabled}
              >
                <BookOpen size={14} />
                {selectedKbCollections.length > 0 && (
                  <span className="toolbar-btn-label">{selectedKbCollections.length} selected</span>
                )}
              </button>
              {kbPickerOpen && (
                <div className="toolbar-kb-dropdown">
                  <div className="toolbar-kb-header">
                    <span className="toolbar-kb-title">Knowledge Bases</span>
                    {selectedKbCollections.length > 0 && (
                      <button
                        className="toolbar-kb-clear"
                        onClick={clearKbSelections}
                      >
                        Clear
                      </button>
                    )}
                  </div>
                  {knowledgeCollections.map((col) => (
                    <div
                      key={col.id}
                      className={`toolbar-kb-item ${selectedKbCollections.includes(col.name) ? 'selected' : ''}`}
                      onClick={() => toggleKbCollection(col.name)}
                    >
                      <span className="toolbar-kb-item-name">{col.name}</span>
                      <span className="toolbar-kb-item-count">{col.entry_count} entries</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      </div>



      {/* <div className="input-footer">
        <div className="input-hint">
          {commandMode
            ? 'Up/Down to navigate, Enter to select, Esc to dismiss'
            : sendOnEnter
              ? 'Enter to send, Shift+Enter for newline'
              : 'Shift+Enter to send'}
        </div>
      </div> */}

      <ConfirmDialog
        open={clearConfirmOpen}
        title="Clear all messages"
        message="This will permanently delete the current conversation. This cannot be undone."
        confirmLabel="Clear"
        variant="danger"
        onConfirm={() => {
          setClearConfirmOpen(false);
          onClearSession?.();
        }}
        onCancel={() => setClearConfirmOpen(false)}
      />
    </div>
  );
}
