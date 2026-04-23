// ---------------------------------------------------------------------------
// ThinkContentBlock -- shared rendering block for think-tag extraction.
//
// Encapsulates the repeated pattern:
//   1. extractThinkTags(content)
//   2. Render ThinkingCard if thinkContent exists
//   3. Render MarkdownSegment if strippedContent is non-empty
//
// Used by both StaticBubble and StreamingBubble in multiple rendering paths.
// ---------------------------------------------------------------------------

import { memo, useMemo } from 'react';
import { MarkdownSegment } from './MessageShared';
import { extractThinkTags } from './messageUtils';
import { ThinkingCard } from './ThinkingCard';

interface ThinkContentBlockProps {
  content: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  markdownComponents: any;
  /** Whether thinking is currently streaming. Defaults to false. */
  isStreaming?: boolean;
  /** CSS class for the outer markdown container. */
  className?: string;
}

/**
 * Renders content that may contain `<think>` tags.
 * Extracts the thinking block, renders it as a ThinkingCard,
 * and renders the remaining content as markdown.
 */
export const ThinkContentBlock = memo(function ThinkContentBlock({
  content,
  markdownComponents,
  isStreaming = false,
  className = 'message-content markdown-body',
}: ThinkContentBlockProps) {
  const think = useMemo(() => extractThinkTags(content), [content]);
  return (
    <>
      {think.thinkContent && (
        <ThinkingCard
          content={think.thinkContent}
          isStreaming={isStreaming || think.isThinkingIncomplete}
        />
      )}
      {think.strippedContent.trim() && (
        <div className={className}>
          <MarkdownSegment text={think.strippedContent} components={markdownComponents} />
        </div>
      )}
    </>
  );
});
