/**
 * AssistantBubble -- thin dispatcher that routes to StreamingBubble or StaticBubble
 * based on whether the message is a live streaming message or a completed/history one.
 *
 * This preserves the original export so ChatPanel.tsx needs zero changes.
 */

import type { Message } from '../../../types';
import type { ToolResultRecord } from '../../../hooks/useChat';
import { StreamingBubble } from './StreamingBubble';
import { StaticBubble } from './StaticBubble';
import './AssistantBubble.css';


export interface AssistantBubbleProps {
  message: Message;
  /** Tool results from progress events (only provided for streaming messages). */
  toolResults?: ToolResultRecord[];
}


export function AssistantBubble({ message, toolResults }: AssistantBubbleProps) {
  const isStreamingMsg = message.id.startsWith('streaming-')
    || message.id.startsWith('cancelled-')
    || message.id.startsWith('error-');

  if (isStreamingMsg) {
    return <StreamingBubble message={message} toolResults={toolResults} />;
  }
  return <StaticBubble message={message} />;
}
