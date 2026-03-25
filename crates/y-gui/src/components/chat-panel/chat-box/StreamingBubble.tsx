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

import type { Message } from '../../../types';
import type { ToolResultRecord } from '../../../hooks/useChat';
import { ToolCallCard } from './ToolCallCard';
import {
  AssistantMessageShell,
  extractThinkTags,
} from './MessageShared';
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

  return (
    <AssistantMessageShell message={message} isStreaming={true}>
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
                isStreaming
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
              isStreaming
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
          isStreaming
        />
      )}

      {message.tool_calls.length > 0 && (
        <div className="message-tool-calls">
          {message.tool_calls.map((tc) => (
            <ToolCallCard key={tc.id} toolCall={tc} />
          ))}
        </div>
      )}
    </AssistantMessageShell>
  );
}
