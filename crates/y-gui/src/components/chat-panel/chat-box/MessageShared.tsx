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

import { useState, useCallback, useRef, lazy, memo, Suspense } from 'react';
import type { FormEvent } from 'react';
import ReactMarkdown from 'react-markdown';
import type { Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {
  Copy,
  Check,
  Share2,
  ThumbsUp,
  ThumbsDown,
  GitBranch,
} from 'lucide-react';
import type { Message } from '../../../types';
import { escapeThinkTags, extractThinkTags } from './messageUtils';
import { formatMessageTime } from '../../../utils/formatMessageTime';
import {
  createFeedbackId,
  submitAssistantFeedback,
  type AssistantFeedbackRating,
} from '../../../lib/assistantFeedback';
import './MessageShared.css';
import './AssistantBubble.css';

const SyntaxHighlighter = lazy(() =>
  import('react-syntax-highlighter').then((module) => ({
    default: module.Prism,
  })),
);

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
      <Suspense fallback={<pre className="code-block-fallback">{children}</pre>}>
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
      </Suspense>
    </div>
  );
});



/* ---- MarkdownSegment ---- */

const REMARK_PLUGINS = [remarkGfm];

function urlTransform(url: string): string {
  return url;
}

/** Render a markdown text segment. */
export const MarkdownSegment = memo(function MarkdownSegment(
  { text, components }: { text: string; components: Components },
) {
  if (!text.trim()) return null;
  return (
    <ReactMarkdown remarkPlugins={REMARK_PLUGINS} urlTransform={urlTransform} components={components}>
      {escapeThinkTags(text)}
    </ReactMarkdown>
  );
});

/* ---- ActionBar ---- */

export interface ActionBarProps {
  /** Text content to copy / share (typically the final answer, think-tags stripped). */
  content: string;
  /** Diagnostics trace receiving explicit evolution feedback. */
  traceId?: string;
  /** Fork the conversation at this message index. */
  onFork?: (messageIndex: number) => void;
  /** 0-based index of this message in the display list (used for forking). */
  messageIndex?: number;
}

/** Action bar shown on hover for assistant / system messages. */
export function ActionBar({ content, traceId, onFork, messageIndex }: ActionBarProps) {
  const [copied, setCopied] = useState(false);
  const [feedback, setFeedback] = useState<AssistantFeedbackRating | null>(null);
  const [pendingFeedback, setPendingFeedback] = useState<AssistantFeedbackRating | null>(null);
  const [showCorrection, setShowCorrection] = useState(false);
  const [correction, setCorrection] = useState('');
  const [feedbackError, setFeedbackError] = useState<string | null>(null);
  const feedbackIds = useRef<Partial<Record<AssistantFeedbackRating, string>>>({});

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

  const handleFork = useCallback(() => {
    if (onFork && messageIndex !== undefined) {
      onFork(messageIndex);
    }
  }, [onFork, messageIndex]);

  const submitFeedback = useCallback(async (
    rating: AssistantFeedbackRating,
    comment?: string,
  ) => {
    if (!traceId || pendingFeedback) return;
    const feedbackId = feedbackIds.current[rating] ?? createFeedbackId();
    feedbackIds.current[rating] = feedbackId;
    setPendingFeedback(rating);
    setFeedbackError(null);
    try {
      await submitAssistantFeedback({ traceId, feedbackId, rating, comment });
      setFeedback(rating);
      setShowCorrection(false);
      setCorrection('');
    } catch (error) {
      setFeedbackError(error instanceof Error ? error.message : String(error));
    } finally {
      setPendingFeedback(null);
    }
  }, [pendingFeedback, traceId]);

  const handleBadFeedback = useCallback(() => {
    if (feedback === 'bad') return;
    setFeedbackError(null);
    setShowCorrection(true);
  }, [feedback]);

  const handleCorrectionSubmit = useCallback((event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void submitFeedback('bad', correction);
  }, [correction, submitFeedback]);

  return (
    <>
      <div className="message-actions">
        <button className="action-btn" onClick={handleCopy} title="Copy message">
          {copied ? <Check size={14} /> : <Copy size={14} />}
          <span className="action-label">{copied ? 'Copied' : 'Copy'}</span>
        </button>

        <button className="action-btn" onClick={handleShare} title="Share message">
          <Share2 size={14} />
          <span className="action-label">Share</span>
        </button>

        {onFork && messageIndex !== undefined && (
          <button className="action-btn" onClick={handleFork} title="Fork conversation from here" aria-label="Fork conversation from here">
            <GitBranch size={14} />
            <span className="action-label">Fork</span>
          </button>
        )}

        {traceId && (
          <>
            <span className="action-divider" />
            <button
              className={`action-btn feedback-btn ${feedback === 'good' ? 'active' : ''}`}
              onClick={() => void submitFeedback('good')}
              title="Good response"
              aria-label="Good response"
              aria-pressed={feedback === 'good'}
              disabled={pendingFeedback !== null || feedback === 'good'}
            >
              <ThumbsUp size={14} />
            </button>

            <button
              className={`action-btn feedback-btn ${feedback === 'bad' ? 'active' : ''}`}
              onClick={handleBadFeedback}
              title="Bad response"
              aria-label="Bad response"
              aria-pressed={feedback === 'bad'}
              disabled={pendingFeedback !== null || feedback === 'bad'}
            >
              <ThumbsDown size={14} />
            </button>
          </>
        )}
      </div>
      {traceId && showCorrection && (
        <form className="feedback-correction" onSubmit={handleCorrectionSubmit}>
          <textarea
            value={correction}
            onChange={(event) => setCorrection(event.target.value)}
            placeholder="What should be corrected?"
            aria-label="Feedback correction"
            rows={2}
            autoFocus
          />
          <div className="feedback-correction-actions">
            <button
              type="button"
              className="action-btn"
              onClick={() => {
                setShowCorrection(false);
                setFeedbackError(null);
              }}
            >
              Cancel
            </button>
            <button
              type="submit"
              className="action-btn feedback-submit"
              disabled={!correction.trim() || pendingFeedback !== null}
            >
              Submit correction
            </button>
          </div>
        </form>
      )}
      {feedbackError && <div className="feedback-error" role="alert">{feedbackError}</div>}
    </>
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
  onFork,
  messageIndex,
  children,
}: {
  message: Message;
  /** Text to copy when the user clicks the copy button.
   *  When omitted, falls back to strippedContent of message.content. */
  copyContent?: string;
  /** Fork the conversation at this message index (assistant messages only). */
  onFork?: (messageIndex: number) => void;
  /** 0-based index of this message in the display list (used for forking). */
  messageIndex?: number;
  children: React.ReactNode;
}) {
  const isSystem = message.role === 'system';

  // Fallback: strip think tags from the raw content.
  const effectiveCopyContent = copyContent ?? extractThinkTags(message.content).strippedContent;
  const traceId = typeof message.metadata?.trace_id === 'string'
    ? message.metadata.trace_id
    : undefined;

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

        <ActionBar
          content={effectiveCopyContent}
          traceId={traceId}
          onFork={onFork}
          messageIndex={messageIndex}
        />

        <div className="message-footer">
          <span className="message-time">
            {formatMessageTime(message.timestamp)}
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
