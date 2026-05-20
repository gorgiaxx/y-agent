import type { Message } from '../types';
import type { ToolResultRecord } from './chatStreamTypes';
import { mergeToolResultMetadata } from './toolResultMetadata';

export function streamingAssistantMessageId(sessionId: string): string {
  return `streaming-${sessionId}`;
}

export function isLiveStreamingAssistantMessage(message: Pick<Message, 'id'>): boolean {
  return message.id.startsWith('streaming-');
}

export function isLocalTerminalAssistantMessage(message: Pick<Message, 'id'>): boolean {
  return message.id.startsWith('cancelled-') || message.id.startsWith('error-');
}

function hasMeaningfulStreamMetadata(metadata: Record<string, unknown>): boolean {
  if (typeof metadata.stream_error === 'string' && metadata.stream_error.length > 0) {
    return true;
  }

  const toolResults = metadata.tool_results;
  if (Array.isArray(toolResults) && toolResults.length > 0) {
    return true;
  }

  const generatedImages = metadata.generated_images;
  if (Array.isArray(generatedImages) && generatedImages.length > 0) {
    return true;
  }

  return typeof metadata.reasoning_content === 'string'
    && metadata.reasoning_content.length > 0;
}

function withoutStreamingFlag(message: Message): Message {
  const copy = { ...message } as Message & { _streaming?: boolean };
  delete copy._streaming;
  return copy;
}

function parseUrlMeta(urlMeta: string): unknown {
  try {
    return JSON.parse(urlMeta);
  } catch {
    return urlMeta;
  }
}

function hasToolResultMetadata(message: Message): boolean {
  const toolResults = message.metadata?.tool_results;
  return Array.isArray(toolResults) && toolResults.length > 0;
}

function hasHistoryRenderingMetadata(message: Message): boolean {
  const iterationTexts = message.metadata?.iteration_texts;
  const generatedImages = message.metadata?.generated_images;
  return hasToolResultMetadata(message)
    || (Array.isArray(iterationTexts) && iterationTexts.length > 0)
    || (Array.isArray(generatedImages) && generatedImages.length > 0)
    || typeof message.metadata?.reasoning_content === 'string';
}

function shouldReplaceLocalTerminalWithBackend(local: Message, backend: Message): boolean {
  if (local.role !== 'assistant' || backend.role !== 'assistant') {
    return false;
  }

  return hasHistoryRenderingMetadata(backend) && !hasToolResultMetadata(local);
}

function appendInterruptedRunningToolResults(
  records: Array<Record<string, unknown>> | undefined,
  toolResults: ToolResultRecord[] | undefined,
): Array<Record<string, unknown>> | undefined {
  const merged = records ? [...records] : [];
  const seen = new Set(
    merged.map((record) => [
      String(record.name ?? ''),
      String(record.arguments ?? ''),
    ].join('::')),
  );

  for (const toolResult of toolResults ?? []) {
    if (toolResult.state !== 'running') {
      continue;
    }
    const key = [toolResult.name, toolResult.arguments ?? ''].join('::');
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    const entry: Record<string, unknown> = {
      name: toolResult.name,
      arguments: toolResult.arguments ?? '',
      success: false,
      duration_ms: toolResult.durationMs,
      result_preview: toolResult.resultPreview || 'Cancelled before result',
    };
    if (toolResult.urlMeta) {
      entry.url_meta = parseUrlMeta(toolResult.urlMeta);
    }
    if (toolResult.metadata) {
      entry.metadata = toolResult.metadata;
    }
    merged.push(entry);
  }

  return merged.length > 0 ? merged : undefined;
}

export function ensureStreamingAssistantMessage(
  messages: Message[],
  sessionId: string,
  timestamp = new Date().toISOString(),
): Message[] {
  const streamingId = streamingAssistantMessageId(sessionId);
  if (messages.some((message) => message.id === streamingId)) {
    return messages;
  }

  return [
    ...messages,
    {
      id: streamingId,
      role: 'assistant' as const,
      content: '',
      timestamp,
      tool_calls: [],
      _streaming: true,
    } as Message,
  ];
}

export function finalizeStreamingAssistantMessage(
  messages: Message[],
  sessionId: string,
  terminalMessageId: string,
  toolResults?: ToolResultRecord[],
  terminalError?: string,
): Message[] {
  const streamingId = streamingAssistantMessageId(sessionId);

  return messages
    .map((message) => {
      if (message.id !== streamingId) {
        return message;
      }

      const metadata = { ...(message.metadata ?? {}) };
      if (terminalError) {
        metadata.stream_error = terminalError;
      }
      const mergedToolResults = mergeToolResultMetadata(
        metadata.tool_results,
        toolResults,
      );
      const terminalToolResults = appendInterruptedRunningToolResults(
        mergedToolResults,
        toolResults,
      );
      if (terminalToolResults) {
        metadata.tool_results = terminalToolResults;
      }

      const shouldPreserve = message.content.length > 0
        || hasMeaningfulStreamMetadata(metadata);
      if (!shouldPreserve) {
        return null;
      }

      const finalized = withoutStreamingFlag(message);
      return {
        ...finalized,
        id: terminalMessageId,
        metadata: Object.keys(metadata).length > 0 ? metadata : undefined,
      } as Message;
    })
    .filter((message): message is Message => message != null);
}

export function mergeBackendMessagesPreservingLocalStreamState(
  backendMessages: Message[],
  cachedMessages: Message[],
): Message[] {
  const backendIndexById = new Map<string, number>();
  for (let i = 0; i < backendMessages.length; i++) {
    backendIndexById.set(backendMessages[i].id, i);
  }

  const consumed = new Set<number>();
  const matchedIdx: number[] = [];
  let lastSeenIdx = -1;

  const findBackendUserByContent = (content: string): number => {
    for (let i = 0; i < backendMessages.length; i++) {
      if (consumed.has(i)) continue;
      const bm = backendMessages[i];
      if (bm.role === 'user' && bm.content === content) {
        consumed.add(i);
        return i;
      }
    }
    return -1;
  };

  const findAdjacentAssistant = (): number => {
    for (let i = lastSeenIdx + 1; i < backendMessages.length; i++) {
      if (consumed.has(i)) continue;
      if (backendMessages[i].role === 'user') break;
      if (backendMessages[i].role === 'assistant') return i;
    }
    return -1;
  };

  for (const message of cachedMessages) {
    const directIdx = backendIndexById.get(message.id);
    if (directIdx !== undefined && !consumed.has(directIdx)) {
      matchedIdx.push(directIdx);
      consumed.add(directIdx);
      lastSeenIdx = Math.max(lastSeenIdx, directIdx);
      continue;
    }

    if (message.role === 'user' && message.id.startsWith('user-')) {
      const idx = findBackendUserByContent(message.content);
      if (idx >= 0) {
        matchedIdx.push(idx);
        lastSeenIdx = Math.max(lastSeenIdx, idx);
      } else {
        matchedIdx.push(-1);
      }
      continue;
    }

    if (isLiveStreamingAssistantMessage(message)) {
      matchedIdx.push(-1);
      continue;
    }

    if (isLocalTerminalAssistantMessage(message)) {
      const adj = findAdjacentAssistant();
      if (adj >= 0
        && shouldReplaceLocalTerminalWithBackend(message, backendMessages[adj])) {
        matchedIdx.push(adj);
        consumed.add(adj);
        lastSeenIdx = Math.max(lastSeenIdx, adj);
      } else {
        if (adj >= 0) consumed.add(adj);
        matchedIdx.push(-1);
      }
      continue;
    }

    matchedIdx.push(-1);
  }

  const merged: Message[] = [];
  const emitted = new Set<number>();
  let cursor = 0;

  for (let ci = 0; ci < cachedMessages.length; ci++) {
    const bIdx = matchedIdx[ci];

    if (bIdx >= 0 && bIdx >= cursor) {
      for (let i = cursor; i < bIdx; i++) {
        if (!consumed.has(i) && !emitted.has(i)) {
          merged.push(backendMessages[i]);
          emitted.add(i);
        }
      }
      merged.push(backendMessages[bIdx]);
      emitted.add(bIdx);
      cursor = bIdx + 1;
    } else if (bIdx >= 0) {
      if (!emitted.has(bIdx)) {
        merged.push(backendMessages[bIdx]);
        emitted.add(bIdx);
      }
    } else {
      merged.push(cachedMessages[ci]);
    }
  }

  for (let i = cursor; i < backendMessages.length; i++) {
    if (!consumed.has(i) && !emitted.has(i)) {
      merged.push(backendMessages[i]);
    }
  }

  return merged;
}
