/**
 * StreamingBubble -- renders a live-streaming assistant message.
 *
 * Handles:
 *  - Live tool results from progress events (toolResults prop)
 *  - Pending tool-call dots animation while buffering incomplete XML tags
 *  - Content that grows on every render as tokens stream in
 *  - Action Segment: groups intermediate tool calls into a collapsible block
 *
 * Shared logic (theme, parsing, segmentation) is delegated to useAssistantBubble.
 * The ThinkContentBlock component handles the repeated think-tag extraction pattern.
 */

import { useMemo } from 'react';
import type { Message } from '../../../types';
import type { ToolResultRecord } from '../../../hooks/useChat';
import { ToolCallCard } from './ToolCallCard';
import {
  AssistantMessageShell,
} from './MessageShared';
import { extractThinkTags } from './messageUtils';
import { ActionCard } from './ActionCard';
import { useAssistantBubble } from './useAssistantBubble';
import { ThinkContentBlock } from './ThinkContentBlock';


export interface StreamingBubbleProps {
  message: Message;
  /** Tool results from progress events (live streaming). */
  toolResults?: ToolResultRecord[];
}


export function StreamingBubble({ message, toolResults }: StreamingBubbleProps) {
  const effectiveContent = message.content;

  const {
    markdownComponents,
    streamResult,
    toolResultsMap,
    actionResult,
  } = useAssistantBubble(effectiveContent, toolResults ?? []);

  // Compute the text to use for the copy button: only the final answer
  // (last LLM call's strippedContent, with think tags removed).
  const copyContent = useMemo(() => {
    if (streamResult && actionResult && actionResult.actions.length > 0) {
      if (actionResult.conclusion?.type === 'text') {
        return extractThinkTags(actionResult.conclusion.text).strippedContent;
      }
      // Still streaming actions, no conclusion yet.
      return '';
    }
    if (streamResult) {
      const textSegs = streamResult.segments.filter((s) => s.type === 'text');
      const last = textSegs[textSegs.length - 1];
      if (last?.type === 'text') {
        return extractThinkTags(last.text).strippedContent;
      }
    }
    return extractThinkTags(effectiveContent).strippedContent;
  }, [streamResult, actionResult, effectiveContent]);

  return (
    <AssistantMessageShell message={message} isStreaming={true} copyContent={copyContent}>
      {streamResult && actionResult && actionResult.actions.length > 0 ? (
        <>
          {/* Preamble: text segments before the first tool call */}
          {actionResult.preamble.map((seg, idx) => {
            if (seg.type !== 'text') return null;
            return (
              <ThinkContentBlock
                key={`preamble-${idx}`}
                content={seg.text}
                markdownComponents={markdownComponents}
              />
            );
          })}
          {/* Action Segment: intermediate tool calls + text */}
          <ActionCard
            segments={actionResult.actions}
            toolCallCount={actionResult.toolCallCount}
            isStreaming={!actionResult.conclusion}
            hasPendingToolCall={streamResult.hasPendingToolCall}
            toolResultsMap={toolResultsMap}
            markdownComponents={markdownComponents}
            segmentIndexOffset={actionResult.preamble.length}
          />
          {/* Conclusion: trailing text from final LLM iteration */}
          {actionResult.conclusion && actionResult.conclusion.type === 'text' && (
            <ThinkContentBlock
              content={actionResult.conclusion.text}
              markdownComponents={markdownComponents}
            />
          )}
        </>
      ) : streamResult ? (
        <div className="message-content">
          {streamResult.segments.map((seg, idx) => {
            if (seg.type === 'text') {
              const think = extractThinkTags(seg.text);
              return (
                <ThinkContentBlock
                  key={`text-${idx}`}
                  content={seg.text}
                  markdownComponents={markdownComponents}
                  isStreaming={think.isThinkingIncomplete}
                  className="markdown-body"
                />
              );
            }
            if (seg.type === 'tool_call') {
              const result = toolResultsMap.get(idx);
              const status = result
                ? (result.success ? 'success' : 'error')
                : 'running';
              return (
                <ToolCallCard
                  key={`tc-${idx}`}
                  toolCall={{
                    id: `tc-${idx}`,
                    name: seg.toolCall.name,
                    arguments: seg.toolCall.arguments,
                  }}
                  status={status}
                  result={result?.resultPreview}
                  durationMs={result?.durationMs}
                />
              );
            }
            return null;
          })}
          {streamResult.hasPendingToolCall && (
            <div className="tool-call-pending">
              <div className="tool-call-pending-dots">
                <span /><span /><span />
              </div>
              <span className="tool-call-pending-text">Calling tool...</span>
            </div>
          )}
        </div>
      ) : (
        <ThinkContentBlock
          content={effectiveContent}
          markdownComponents={markdownComponents}
        />
      )}

      {/* Native tool call results (from progress events, no XML parsing). */}
      {!streamResult && toolResults && toolResults.length > 0 && (
        <div className="message-tool-calls">
          {toolResults.map((tr, idx) => (
            <ToolCallCard
              key={`native-tc-${idx}`}
              toolCall={{ id: `native-${idx}`, name: tr.name, arguments: tr.arguments ?? '' }}
              status={tr.success ? 'success' : 'error'}
              result={tr.resultPreview}
              durationMs={tr.durationMs}
            />
          ))}
        </div>
      )}

      {/* Native tool calls from message object (post-completion). */}
      {!streamResult && message.tool_calls.length > 0 && (
        <div className="message-tool-calls">
          {message.tool_calls.map((tc) => (
            <ToolCallCard key={tc.id} toolCall={tc} />
          ))}
        </div>
      )}
    </AssistantMessageShell>
  );
}
