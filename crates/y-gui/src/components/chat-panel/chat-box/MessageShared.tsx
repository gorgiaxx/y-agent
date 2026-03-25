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

import { useState, useCallback } from 'react';
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
import { ThinkingCard } from './ThinkingCard';
import { MermaidBlock } from './MermaidBlock';
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
export function CodeBlock({
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
}

/* ---- makeMarkdownComponents ---- */

/** Shared markdown renderer config -- needs theme to pick syntax style. */
export function makeMarkdownComponents(codeThemeStyle: Record<string, React.CSSProperties>) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const components: any = {
    code({ className, children, ...props }: { className?: string; children?: React.ReactNode; [key: string]: unknown }) {
      const match = /language-(\w+)/.exec(className || '');
      const codeText = String(children).replace(/\n$/, '');

      if (match && match[1] === 'mermaid') {
        return <MermaidBlock code={codeText} />;
      }

      if (match) {
        return (
          <CodeBlock language={match[1]} themeStyle={codeThemeStyle}>{codeText}</CodeBlock>
        );
      }

      // Inline code
      return (
        <code className="inline-code" {...props}>
          {children}
        </code>
      );
    },
  };
  return components;
}

/* ---- escapeThinkTags ---- */

/**
 * Escape literal `<think>` / `</think>` tags in text so ReactMarkdown does
 * not interpret them as HTML elements. After extraction, any remaining
 * `<think>` in the content is just regular text the LLM happened to mention.
 */
export function escapeThinkTags(text: string): string {
  return text
    .replace(/<think>/g, '&lt;think&gt;')
    .replace(/<\/think>/g, '&lt;/think&gt;');
}

/* ---- MarkdownSegment ---- */

/** Render a markdown text segment. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function MarkdownSegment({ text, components }: { text: string; components: any }) {
  if (!text.trim()) return null;
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
      {escapeThinkTags(text)}
    </ReactMarkdown>
  );
}

/* ---- extractThinkTags ---- */

/**
 * Minimum character count for completed `<think>` content to be treated as
 * genuine reasoning. Content shorter than this (e.g. `<think>/</think>` where
 * the LLM is just mentioning the tag syntax) is treated as a false positive
 * and returned as part of the normal content.
 *
 * This guard only applies to COMPLETED think blocks (both tags present).
 * Still-streaming blocks (no closing tag) are always returned since the
 * content is still growing.
 */
const MIN_THINK_CONTENT_LENGTH = 5;

/**
 * Extract `<think>...</think>` tags from message content.
 *
 * Some models (e.g. DeepSeek, QwQ) embed chain-of-thought inside `<think>` tags
 * in the main content rather than sending a separate `reasoning` field.
 *
 * Returns the extracted thinking text and the remaining content with tags stripped.
 * If the closing `</think>` tag is missing, the content after `<think>` is treated
 * as still-streaming thinking content.
 */
export function extractThinkTags(content: string): {
  thinkContent: string | null;
  strippedContent: string;
  isThinkingIncomplete: boolean;
} {
  const openTag = '<think>';
  const closeTag = '</think>';

  const openIdx = content.indexOf(openTag);
  if (openIdx != 0) {
    return { thinkContent: null, strippedContent: content, isThinkingIncomplete: false };
  }
  const afterOpen = openIdx + openTag.length;
  const closeIdx = content.indexOf(closeTag, afterOpen);

  if (closeIdx < 0) {
    // The <think> tag is not closed -- still streaming thinking content.
    const thinkContent = content.slice(afterOpen).trim();
    const strippedContent = content.slice(0, openIdx).trim();
    return {
      thinkContent: thinkContent || null,
      strippedContent,
      isThinkingIncomplete: true,
    };
  }

  // Complete <think>...</think> block found.
  const thinkContent = content.slice(afterOpen, closeIdx).trim();

  // Guard: if the content between tags is too short, it is likely the LLM
  // mentioning the tag syntax (e.g. `<think>/</think>`) rather than embedding
  // actual reasoning. Treat such cases as normal content (no extraction).
  if (thinkContent.length < MIN_THINK_CONTENT_LENGTH) {
    return { thinkContent: null, strippedContent: content, isThinkingIncomplete: false };
  }

  // Strip the entire <think>...</think> block from the content.
  const strippedContent = (
    content.slice(0, openIdx) + content.slice(closeIdx + closeTag.length)
  ).trim();

  return {
    thinkContent: thinkContent || null,
    strippedContent,
    isThinkingIncomplete: false,
  };
}

/* ---- ActionBar ---- */

/** Action bar shown on hover for assistant / system messages. */
export function ActionBar({ content }: { content: string }) {
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
  isStreaming,
  children,
}: {
  message: Message;
  isStreaming: boolean;
  children: React.ReactNode;
}) {
  const isSystem = message.role === 'system';

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

        {/* Reasoning/thinking block at the top of assistant messages */}
        {/* Source 1: metadata.reasoning_content (from stream_reasoning_delta events) */}
        {typeof message.metadata?.reasoning_content === 'string' && (
          <ThinkingCard
            content={message.metadata.reasoning_content}
            isStreaming={isStreaming && !message.metadata?._reasoningDoneTs}
            durationMs={(message.metadata?.reasoning_duration_ms ?? message.metadata?._reasoningDurationMs) as number | undefined}
          />
        )}
        {/* <think> tag extraction is handled by child components
            (StreamingBubble / StaticBubble) in each rendering path,
            so the ThinkingCard appears in the correct position
            (e.g. after ActionCard in the conclusion). */}

        {children}

        <ActionBar content={message.content} />

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
