import { useRef, useCallback, useImperativeHandle, forwardRef } from 'react';
import { X, Loader2 } from 'lucide-react';
import type { Attachment } from '../../../types';
import './InputArea.css';

/** Data attribute used to identify skill mention tokens in the contenteditable. */
const SKILL_ATTR = 'data-skill-name';

// ---------------------------------------------------------------------------
// DOM utilities for the contenteditable div
// ---------------------------------------------------------------------------

function walkDom(
  root: HTMLDivElement,
  onSkill?: (name: string) => void,
): string {
  let text = '';

  function walk(node: Node) {
    if (node.nodeType === Node.TEXT_NODE) {
      text += node.textContent || '';
    } else if (node.nodeType === Node.ELEMENT_NODE) {
      const element = node as HTMLElement;
      const skillName = element.getAttribute(SKILL_ATTR);
      if (skillName) {
        onSkill?.(skillName);
        return;
      }
      if (element.tagName === 'BR') {
        text += '\n';
      } else {
        if (element.tagName === 'DIV' && element !== root) {
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

  for (const child of Array.from(root.childNodes)) {
    walk(child);
  }
  return text;
}

function extractContentFromDom(el: HTMLDivElement): { text: string; skills: string[] } {
  const skills: string[] = [];
  const text = walkDom(el, (name) => {
    if (!skills.includes(name)) skills.push(name);
  });
  return { text, skills };
}

function getPlainTextFromDom(el: HTMLDivElement): string {
  return walkDom(el);
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

// ---------------------------------------------------------------------------
// Imperative handle
// ---------------------------------------------------------------------------

export interface ContentEditableInputHandle {
  extractContent: () => { text: string; skills: string[] };
  getPlainText: () => string;
  setText: (text: string) => void;
  clear: () => void;
  insertSkillMention: (skillName: string) => void;
  hasContent: () => boolean;
  focus: () => void;
  removeSlashCommandText: () => void;
  placeCursorAtEnd: () => void;
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface ContentEditableInputProps {
  disabled: boolean;
  translating: boolean;
  isCompacting: boolean;
  attachments: Attachment[];
  onRemoveAttachment: (id: string) => void;
  onInput: (plainText: string) => void;
  onPaste: (e: React.ClipboardEvent) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  onCompositionEnd: () => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export const ContentEditableInput = forwardRef<ContentEditableInputHandle, ContentEditableInputProps>(
  function ContentEditableInput(
    {
      disabled,
      translating,
      isCompacting,
      attachments,
      onRemoveAttachment,
      onInput,
      onPaste,
      onKeyDown,
      onCompositionEnd,
    },
    ref,
  ) {
    const editableRef = useRef<HTMLDivElement>(null);

    // -- Imperative API --
    useImperativeHandle(ref, () => ({
      extractContent() {
        if (!editableRef.current) return { text: '', skills: [] };
        return extractContentFromDom(editableRef.current);
      },
      getPlainText() {
        if (!editableRef.current) return '';
        return getPlainTextFromDom(editableRef.current);
      },
      setText(text: string) {
        if (!editableRef.current) return;
        editableRef.current.textContent = text;
      },
      clear() {
        if (!editableRef.current) return;
        editableRef.current.innerHTML = '';
      },
      insertSkillMention(skillName: string) {
        if (!editableRef.current) return;

        // Check if skill is already mentioned.
        const existing = editableRef.current.querySelector(`[${SKILL_ATTR}="${skillName}"]`);
        if (existing) {
          placeCursorAtEnd(editableRef.current);
          return;
        }

        const mention = createSkillMention(skillName);
        editableRef.current.appendChild(mention);

        // Add a trailing space so the cursor has somewhere to go.
        const space = document.createTextNode('\u00A0');
        editableRef.current.appendChild(space);

        placeCursorAtEnd(editableRef.current);
        editableRef.current.focus();
      },
      hasContent() {
        if (!editableRef.current) return false;
        return getPlainTextFromDom(editableRef.current).trim().length > 0;
      },
      focus() {
        editableRef.current?.focus();
      },
      removeSlashCommandText() {
        if (!editableRef.current) return;
        const textNodes = Array.from(editableRef.current.childNodes).filter(
          (n) => n.nodeType === Node.TEXT_NODE,
        );
        for (const tn of textNodes) {
          const t = tn.textContent || '';
          if (t.startsWith('/')) {
            tn.textContent = t.replace(/^\/\S*\s?/, '');
          }
        }
      },
      placeCursorAtEnd() {
        if (editableRef.current) placeCursorAtEnd(editableRef.current);
      },
    }));

    const handleInput = useCallback(() => {
      if (!editableRef.current) return;
      const plainText = getPlainTextFromDom(editableRef.current);
      onInput(plainText);
    }, [onInput]);

    return (
      <div className="input-content">
        {/* Attachment preview strip */}
        {attachments.length > 0 && (
          <div className="attachment-preview-strip">
            {attachments.map((att) => (
              <div key={att.id} className="attachment-thumb">
                <img
                  src={`data:${att.mime_type};base64,${att.base64_data}`}
                  alt={att.filename}
                  className="attachment-thumb-img"
                />
                <button
                  className="attachment-remove-btn"
                  onClick={() => onRemoveAttachment(att.id)}
                  title={`Remove ${att.filename}`}
                  aria-label={`Remove ${att.filename}`}
                >
                  <X size={10} />
                </button>
              </div>
            ))}
          </div>
        )}
        <div
          ref={editableRef}
          className="input-editable"
          contentEditable={!disabled && !translating}
          onInput={handleInput}
          onPaste={onPaste}
          onKeyDown={onKeyDown}
          onCompositionEnd={onCompositionEnd}
          data-placeholder={isCompacting ? 'Compacting context, please wait...' : disabled ? 'Waiting for response...' : 'Type a message... (/ for commands), Enter to send, Shift+Enter for newline)'}
          role="textbox"
          aria-multiline="true"
          suppressContentEditableWarning
        />

        {translating && (
          <div className="translating-overlay" title="Translating...">
            <Loader2 size={14} className="translating-spinner" />
            <span className="translating-label">Translating...</span>
          </div>
        )}
      </div>
    );
  },
);
