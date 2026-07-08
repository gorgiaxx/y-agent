/**
 * AssistantBubble -- thin dispatcher that routes to StreamingBubble or StaticBubble
 * based on whether the message is a live streaming message or a completed/history one.
 *
 * This preserves the original export so ChatPanel.tsx needs zero changes.
 */

import { memo } from 'react';
import type { Message } from '../../../types';
import type { ToolResultRecord } from '../../../hooks/chatStreamTypes';
import type { InterleavedSegment } from '../../../hooks/useInterleavedSegments';
import { isLiveStreamingAssistantMessage } from '../../../hooks/chatStreamingMessages';
import { StreamingBubble } from './StreamingBubble';
import { StaticBubble } from './StaticBubble';
import './AssistantBubble.css';


export interface AssistantBubbleProps {
  message: Message;
  /** Tool results from progress events (only provided for streaming messages). */
  toolResults?: ToolResultRecord[];
  /** Lazy getter for event-ordered segments (only called for streaming messages). */
  getStreamSegments?: () => InterleavedSegment[] | null;
  /** Retry the turn that produced this errored reply (static/history messages only). */
  onRetry?: () => void;
  /** Fork the conversation at this message index (static/history messages only). */
  onFork?: (messageIndex: number) => void;
  /** 0-based index of this message in the display list (used for forking). */
  messageIndex?: number;
}


export const AssistantBubble = memo(function AssistantBubble(
  { message, toolResults, getStreamSegments, onRetry, onFork, messageIndex }: AssistantBubbleProps,
) {
  if (isLiveStreamingAssistantMessage(message)) {
    const streamSegments = getStreamSegments?.() ?? null;
    return <StreamingBubble message={message} toolResults={toolResults} streamSegments={streamSegments} />;
  }
  return <StaticBubble message={message} onRetry={onRetry} onFork={onFork} messageIndex={messageIndex} />;
});
