import { useState, useCallback } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import {
  Copy,
  Check,
  Share2,
  ThumbsUp,
  ThumbsDown,
  Pencil,
  Undo2,
  RefreshCw,
} from 'lucide-react';
import type { Message } from '../types';
import { ToolCallCard } from './ToolCallCard';
import './MessageBubble.css';

interface MessageBubbleProps {
  message: Message;
  onEdit?: (content: string) => void;
  onUndo?: (messageId: string) => void;
  onResend?: (content: string) => void;
}

/** CSS-styled letter avatar instead of emoji. */
function Avatar({ role }: { role: string }) {
  const letter = role === 'user' ? 'U' : role === 'system' ? 'S' : 'A';
  return (
    <div className={`message-avatar avatar-${role}`}>
      {letter}
    </div>
  );
}

/** Fenced code block with language label and copy button. */
function CodeBlock({
  language,
  children,
}: {
  language: string;
  children: string;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(children).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [children]);

  return (
    <div className="code-block-wrapper">
      <div className="code-block-header">
        <span className="code-block-lang">{language || 'text'}</span>
        <button
          className="code-block-copy"
          onClick={handleCopy}
          title="Copy code"
        >
          {copied ? <Check size={14} /> : <Copy size={14} />}
        </button>
      </div>
      <SyntaxHighlighter
        style={oneDark}
        language={language || 'text'}
        PreTag="div"
        customStyle={{
          margin: 0,
          borderRadius: 0,
          fontSize: '13px',
        }}
      >
        {children}
      </SyntaxHighlighter>
    </div>
  );
}

/** Action bar shown on hover for assistant / system messages. */
function ActionBar({ content }: { content: string }) {
  const [copied, setCopied] = useState(false);
  const [feedback, setFeedback] = useState<'good' | 'bad' | null>(null);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(content).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [content]);

  const handleShare = useCallback(() => {
    if (navigator.share) {
      navigator.share({ text: content }).catch(() => {});
    } else {
      // Fallback: copy to clipboard
      navigator.clipboard.writeText(content);
    }
  }, [content]);

  return (
    <div className="message-actions">
      <button className="action-btn" onClick={handleCopy} title="Copy message">
        {copied ? <Check size={14} /> : <Copy size={14} />}
        <span className="action-label">{copied ? 'Copied' : 'Copy'}</span>
      </button>

      <button className="action-btn" onClick={handleShare} title="Share message">
        <Share2 size={14} />
        <span className="action-label">Share</span>
      </button>

      <span className="action-divider" />

      <button
        className={`action-btn feedback-btn ${feedback === 'good' ? 'active' : ''}`}
        onClick={() => setFeedback(feedback === 'good' ? null : 'good')}
        title="Good response"
      >
        <ThumbsUp size={14} />
      </button>

      <button
        className={`action-btn feedback-btn ${feedback === 'bad' ? 'active' : ''}`}
        onClick={() => setFeedback(feedback === 'bad' ? null : 'bad')}
        title="Bad response"
      >
        <ThumbsDown size={14} />
      </button>
    </div>
  );
}

/** Action bar shown on hover for user messages: Copy, Edit, Resend, Undo. */
function UserActionBar({
  content,
  messageId,
  onEdit,
  onUndo,
  onResend,
}: {
  content: string;
  messageId: string;
  onEdit?: (content: string) => void;
  onUndo?: (messageId: string) => void;
  onResend?: (content: string) => void;
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
      console.warn('[MessageBubble] Edit handler not yet connected');
    }
  }, [content, onEdit]);

  const handleUndo = useCallback(() => {
    if (onUndo) {
      onUndo(messageId);
    } else {
      console.warn('[MessageBubble] Undo handler not yet connected');
    }
  }, [messageId, onUndo]);

  const handleResend = useCallback(() => {
    if (onResend) {
      onResend(content);
    } else {
      console.warn('[MessageBubble] Resend handler not yet connected');
    }
  }, [content, onResend]);

  return (
    <div className="message-actions user-action-bar">
      <button className="action-btn" onClick={handleCopy} title="Copy message" aria-label="Copy message">
        {copied ? <Check size={14} /> : <Copy size={14} />}
        <span className="action-label">{copied ? 'Copied' : 'Copy'}</span>
      </button>

      <button className="action-btn" onClick={handleEdit} title="Edit message" aria-label="Edit message">
        <Pencil size={14} />
        <span className="action-label">Edit</span>
      </button>

      <button className="action-btn" onClick={handleResend} title="Resend message" aria-label="Resend message">
        <RefreshCw size={14} />
        <span className="action-label">Resend</span>
      </button>

      <button className="action-btn" onClick={handleUndo} title="Undo to this point" aria-label="Undo to this point">
        <Undo2 size={14} />
        <span className="action-label">Undo</span>
      </button>
    </div>
  );
}

export function MessageBubble({ message, onEdit, onUndo, onResend }: MessageBubbleProps) {
  const isUser = message.role === 'user';
  const isSystem = message.role === 'system';

  // Phase 3: Keyboard shortcut handler for user messages.
  const handleBubbleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (!isUser) return;
      if (e.altKey && e.key === 'e') {
        e.preventDefault();
        if (onEdit) onEdit(message.content);
      } else if (e.altKey && e.key === 'z') {
        e.preventDefault();
        if (onUndo) onUndo(message.id);
      }
    },
    [isUser, message.content, message.id, onEdit, onUndo],
  );

  return (
    <div
      className={`message-bubble ${message.role}`}
      tabIndex={isUser ? 0 : undefined}
      onKeyDown={isUser ? handleBubbleKeyDown : undefined}
      aria-label={isUser ? `Your message: ${message.content.slice(0, 60)}` : undefined}
    >
      <Avatar role={message.role} />
      <div className="message-body">
        <div className="message-header">
          <span className="message-role">
            {isUser ? 'You' : isSystem ? 'System' : 'Assistant'}
          </span>
          {message.model && (
            <span className="message-model">{message.model}</span>
          )}
        </div>

        {/* User messages render as plain styled text */}
        {isUser ? (
          <div className="message-content user-plain">
            {message.content}
          </div>
        ) : (
          /* Assistant / system messages render as markdown */
          <div className="message-content markdown-body">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={{
                code({ className, children, ...props }) {
                  const match = /language-(\w+)/.exec(className || '');
                  const codeText = String(children).replace(/\n$/, '');

                  if (match) {
                    return (
                      <CodeBlock language={match[1]}>{codeText}</CodeBlock>
                    );
                  }

                  // Inline code
                  return (
                    <code className="inline-code" {...props}>
                      {children}
                    </code>
                  );
                },
              }}
            >
              {message.content}
            </ReactMarkdown>
          </div>
        )}

        {message.tool_calls.length > 0 && (
          <div className="message-tool-calls">
            {message.tool_calls.map((tc) => (
              <ToolCallCard key={tc.id} toolCall={tc} />
            ))}
          </div>
        )}

        {/* Action bar */}
        {isUser
          ? <UserActionBar content={message.content} messageId={message.id} onEdit={onEdit} onUndo={onUndo} onResend={onResend} />
          : <ActionBar content={message.content} />
        }

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
