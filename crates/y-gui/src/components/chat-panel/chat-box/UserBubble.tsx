/**
 * UserBubble -- self-contained component for rendering user messages.
 *
 * Renders:
 *  - right-aligned avatar
 *  - plain-text content bubble with optional skill tags
 *  - UserActionBar (Copy / Edit / Resend / Undo) on hover
 *  - footer (timestamp, tokens, cost)
 *  - keyboard shortcuts (Alt+E = edit, Alt+Z = undo)
 */

import { useState, useCallback } from 'react';
import {
  Copy,
  Check,
  Pencil,
  Undo2,
  RefreshCw,
  Puzzle,
  GitBranch,
} from 'lucide-react';
import type { Message, Attachment } from '../../../types';
import { logger } from '../../../lib';
import { Avatar } from './MessageShared';
import './UserBubble.css';


export interface UserBubbleProps {
  message: Message;
  /** 0-based index of this message in the display list. */
  messageIndex?: number;
  onEdit?: (content: string) => void;
  onUndo?: (messageId: string) => void;
  onResend?: (content: string) => void;
  onFork?: (messageIndex: number) => void;
  /** When true, action bar buttons are disabled (e.g. task is running). */
  disabled?: boolean;
}


/** Action bar shown on hover for user messages: Copy, Edit, Resend, Undo, Fork. */
function UserActionBar({
  content,
  messageId,
  messageIndex,
  onEdit,
  onUndo,
  onResend,
  onFork,
  disabled = false,
}: {
  content: string;
  messageId: string;
  messageIndex?: number;
  onEdit?: (content: string) => void;
  onUndo?: (messageId: string) => void;
  onResend?: (content: string) => void;
  onFork?: (messageIndex: number) => void;
  disabled?: boolean;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(content).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [content]);

  const handleEdit = useCallback(() => {
    if (onEdit) {
      onEdit(content);
    } else {
      logger.warn('[UserBubble] Edit handler not yet connected');
    }
  }, [content, onEdit]);

  const handleUndo = useCallback(() => {
    if (onUndo) {
      onUndo(messageId);
    } else {
      logger.warn('[UserBubble] Undo handler not yet connected');
    }
  }, [messageId, onUndo]);

  const handleResend = useCallback(() => {
    if (onResend) {
      onResend(content);
    } else {
      logger.warn('[UserBubble] Resend handler not yet connected');
    }
  }, [content, onResend]);

  const handleFork = useCallback(() => {
    if (onFork && messageIndex !== undefined) {
      onFork(messageIndex);
    }
  }, [messageIndex, onFork]);

  return (
    <div className={`message-actions user-action-bar${disabled ? ' disabled' : ''}`}>
      <button className="action-btn" onClick={handleCopy} title="Copy message" aria-label="Copy message" disabled={disabled}>
        {copied ? <Check size={14} /> : <Copy size={14} />}
        <span className="action-label">{copied ? 'Copied' : 'Copy'}</span>
      </button>

      <button className="action-btn" onClick={handleEdit} title="Edit message" aria-label="Edit message" disabled={disabled}>
        <Pencil size={14} />
        <span className="action-label">Edit</span>
      </button>

      <button className="action-btn" onClick={handleResend} title="Resend message" aria-label="Resend message" disabled={disabled}>
        <RefreshCw size={14} />
        <span className="action-label">Resend</span>
      </button>

      <button className="action-btn" onClick={handleUndo} title="Undo to this point" aria-label="Undo to this point" disabled={disabled}>
        <Undo2 size={14} />
        <span className="action-label">Undo</span>
      </button>

      {onFork && messageIndex !== undefined && (
        <button className="action-btn" onClick={handleFork} title="Fork conversation from here" aria-label="Fork conversation from here" disabled={disabled}>
          <GitBranch size={14} />
          <span className="action-label">Fork</span>
        </button>
      )}
    </div>
  );
}


export function UserBubble({ message, messageIndex, onEdit, onUndo, onResend, onFork, disabled }: UserBubbleProps) {
  const handleBubbleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (disabled) return;
      if (e.altKey && e.key === 'e') {
        e.preventDefault();
        if (onEdit) onEdit(message.content);
      } else if (e.altKey && e.key === 'z') {
        e.preventDefault();
        if (onUndo) onUndo(message.id);
      }
    },
    [message.content, message.id, onEdit, onUndo, disabled],
  );

  return (
    <div
      className="message-bubble user"
      tabIndex={0}
      onKeyDown={handleBubbleKeyDown}
      aria-label={`Your message: ${message.content.slice(0, 60)}`}
    >
      <Avatar role="user" />
      <div className="message-body">
        <div className="message-header">
          <span className="message-role">You</span>
          {message.model && (
            <span className="message-model">{message.model}</span>
          )}
        </div>

        <div className="message-content user-plain">
          {message.skills && message.skills.length > 0 && (
            <div className="message-skill-tags">
              {message.skills.map((s) => (
                <span key={s} className="message-skill-tag">
                  <Puzzle size={11} className="message-skill-tag-icon" />
                  {s}
                </span>
              ))}
            </div>
          )}
          {(() => {
            const attachments = (message.metadata?.attachments ?? []) as Attachment[];
            return attachments.length > 0 ? (
              <div className="message-attachments">
                {attachments.map((att) => (
                  <img
                    key={att.id}
                    src={`data:${att.mime_type};base64,${att.base64_data}`}
                    alt={att.filename}
                    className="message-attachment-thumb"
                    title={att.filename}
                  />
                ))}
              </div>
            ) : null;
          })()}
          {message.content}
        </div>

        <UserActionBar
          content={message.content}
          messageId={message.id}
          messageIndex={messageIndex}
          onEdit={onEdit}
          onUndo={onUndo}
          onResend={onResend}
          onFork={onFork}
          disabled={disabled}
        />

        <div className="message-footer">
          <span className="message-time">
            {new Date(message.timestamp).toLocaleTimeString([], {
              hour: '2-digit',
              minute: '2-digit',
            })}
          </span>
          {message.tokens && (
            <span className="message-tokens">
              {message.tokens.input + message.tokens.output} tokens
            </span>
          )}
          {message.cost !== undefined && message.cost > 0 && (
            <span className="message-cost">
              ${message.cost.toFixed(4)}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
