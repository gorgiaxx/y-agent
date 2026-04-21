import React from 'react';
import { platform } from '../../../lib/platform';
import { CodeBlock } from './MessageShared';
import { MermaidBlock } from './MermaidBlock';

/* ---- makeMarkdownComponents ---- */

function isAbsoluteWebUrl(href: unknown): href is string {
  return typeof href === 'string' && /^https?:\/\//i.test(href);
}

/** Shared markdown renderer config -- needs theme to pick syntax style. */
export function makeMarkdownComponents(codeThemeStyle: Record<string, React.CSSProperties>) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const components: any = {
    a({
      href,
      children,
      node,
      ...props
    }: {
      href?: string;
      children?: React.ReactNode;
      node?: unknown;
      [key: string]: unknown;
    }) {
      void node;

      const isWebUrl = isAbsoluteWebUrl(href);
      const handleClick = (event: React.MouseEvent<HTMLAnchorElement>) => {
        if (!isWebUrl) return;

        event.preventDefault();
        event.stopPropagation();
        platform.openUrl(href).catch((err) =>
          console.error('[MessageMarkdown] failed to open URL:', href, err),
        );
      };

      return (
        <a
          {...props}
          href={href}
          target={isWebUrl ? '_blank' : undefined}
          rel={isWebUrl ? 'noopener noreferrer' : undefined}
          onClick={handleClick}
        >
          {children}
        </a>
      );
    },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    code({ className, children, node, ...props }: { className?: string; children?: React.ReactNode; node?: any; [key: string]: unknown }) {
      const match = /language-(\w+)/.exec(className || '');
      const codeText = String(children).replace(/\n$/, '');

      // Detect fenced code blocks: react-markdown wraps them in <pre><code>.
      // When no language is specified, className is absent, but the parent
      // <pre> element still exists. Check for it to avoid falling through
      // to the inline-code path.
      const isBlock = match != null
        || node?.position?.start?.line !== node?.position?.end?.line
        || (node?.properties?.className != null)
        || (typeof node?.tagName === 'string'
            && node?.parent?.tagName === 'pre');

      // Fallback: if none of the node heuristics fired, check whether
      // the raw text itself spans multiple lines -- this reliably signals
      // a fenced block even for single-backtick edge cases.
      const isFencedBlock = isBlock || codeText.includes('\n');

      if (match && match[1] === 'mermaid') {
        return <MermaidBlock code={codeText} />;
      }

      if (match) {
        return (
          <CodeBlock language={match[1]} themeStyle={codeThemeStyle}>{codeText}</CodeBlock>
        );
      }

      // Fenced code block without a language specifier
      if (isFencedBlock) {
        return (
           <CodeBlock language="" themeStyle={codeThemeStyle}>{codeText}</CodeBlock>
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
  // Allow leading whitespace before the <think> tag.
  // Only extract when the tag appears at the effective start of the content
  // (i.e. nothing but whitespace before it).
  if (openIdx < 0 || content.slice(0, openIdx).trim().length > 0) {
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
  // mentioning the tag syntax (e.g. `<think>/<think>`) rather than embedding
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
