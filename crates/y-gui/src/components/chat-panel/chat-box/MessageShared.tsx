/**
 * MessageShared -- role-agnostic helpers shared by UserBubble and AssistantBubble.
 *
 * Exports:
 *   Avatar       -- letter-based avatar circle
 *   CodeBlock    -- fenced code block with language label and copy button
 *   MarkdownSegment -- renders a markdown text segment via ReactMarkdown
 *   makeMarkdownComponents -- factory for ReactMarkdown `components` prop
 */

import { useState, useCallback } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { Copy, Check } from 'lucide-react';
import { MermaidBlock } from './MermaidBlock';
import './MessageShared.css';

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

/* ---- MarkdownSegment ---- */

/** Render a markdown text segment. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function MarkdownSegment({ text, components }: { text: string; components: any }) {
  if (!text.trim()) return null;
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
      {text}
    </ReactMarkdown>
  );
}
