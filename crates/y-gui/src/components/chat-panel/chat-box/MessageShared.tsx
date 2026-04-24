/**
 * MessageShared -- role-agnostic helpers shared by UserBubble, StreamingBubble,
 * and StaticBubble.
 *
 * Exports:
 *   Avatar       -- letter-based avatar circle
 *   CodeBlock    -- fenced code block with language label and copy button
 *   MarkdownSegment -- renders a markdown text segment via ReactMarkdown
 *   makeMarkdownComponents -- factory for ReactMarkdown `components` prop
 *   extractThinkTags -- extract <think>...</think> from content
 *   ActionBar    -- copy / share / feedback bar for assistant messages
 *   AssistantMessageShell -- shared layout wrapper for assistant messages
 */

import { useState, useCallback, memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import {
  Copy,
  Check,
  Share2,
  ThumbsUp,
  ThumbsDown,
} from 'lucide-react';
import type { Message } from '../../../types';
import { escapeThinkTags, extractThinkTags } from './messageUtils';
import './MessageShared.css';
import './AssistantBubble.css';


/* ---- Avatar ---- */

/** CSS-styled letter avatar instead of emoji. */
export function Avatar({ role }: { role: string }) {
  const letter = role === 'user' ? 'U' : role === 'system' ? 'S' : 'A';
  return (
    <div className={`message-avatar avatar-${role}`}>
      {letter}
    </div>
  );
}

/* ---- CodeBlock ---- */

/** Fenced code block with language label and copy button. */
export const CodeBlock = memo(function CodeBlock({
  language,
  children,
  themeStyle,
}: {
  language: string;
  children: string;
  themeStyle: Record<string, React.CSSProperties>;
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
        style={themeStyle}
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
});



/* ---- MarkdownSegment ---- */

const REMARK_PLUGINS = [remarkGfm];

/** Render a markdown text segment. */
export const MarkdownSegment = memo(function MarkdownSegment(
  { text, components }: { text: string; components: Record<string, unknown> },
) {
  if (!text.trim()) return null;
  return (
    <ReactMarkdown remarkPlugins={REMARK_PLUGINS} components={components}>
      {escapeThinkTags(text)}
    </ReactMarkdown>
  );
});

/* ---- ActionBar ---- */

export interface ActionBarProps {
  /** Text content to copy / share (typically the final answer, think-tags stripped). */
  content: string;
  /** Session ID for future feedback submission. */
  sessionId?: string;
}

/** Action bar shown on hover for assistant / system messages. */
export function ActionBar({ content }: ActionBarProps) {
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

/* ---- AssistantMessageShell ---- */

/**
 * Shared layout wrapper for assistant / system messages.
 *
 * Renders: avatar, header (role + model), thinking block(s), children (content),
 * legacy tool_calls, action bar, and footer (timestamp, tokens, cost).
 */
export function AssistantMessageShell({
  message,
  copyContent,
  children,
}: {
  message: Message;
  /** Text to copy when the user clicks the copy button.
   *  When omitted, falls back to strippedContent of message.content. */
  copyContent?: string;
  children: React.ReactNode;
}) {
  const isSystem = message.role === 'system';

  // Fallback: strip think tags from the raw content.
  const effectiveCopyContent = copyContent ?? extractThinkTags(message.content).strippedContent;

  return (
    <div className={`message-bubble ${message.role}`}>
      <Avatar role={message.role} />
      <div className="message-body">
        <div className="message-header">
          <span className="message-role">
            {isSystem ? 'System' : 'Assistant'}
          </span>
          {message.model && (
            <span className="message-model">{message.model}</span>
          )}
        </div>

        {/* Reasoning/thinking is rendered inline via segments in
            StreamingBubble / StaticBubble. <think> tag extraction is
            handled by ThinkContentBlock. */}

        {children}

        <ActionBar content={effectiveCopyContent} />

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
