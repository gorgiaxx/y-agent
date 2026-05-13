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
  const backendById = new Map(
    backendMessages.map((message) => [message.id, message]),
  );
  const usedBackendIds = new Set<string>();
  const merged: Message[] = [];

  const pushBackendMessage = (message: Message) => {
    if (usedBackendIds.has(message.id)) {
      return;
    }
    usedBackendIds.add(message.id);
    merged.push(message);
  };

  const takeMatchingBackendUser = (content: string): Message | null => {
    const match = backendMessages.find((message) =>
      message.role === 'user'
      && message.content === content
      && !usedBackendIds.has(message.id),
    );
    if (!match) {
      return null;
    }
    usedBackendIds.add(match.id);
    return match;
  };

  for (const message of cachedMessages) {
    const backendMessage = backendById.get(message.id);
    if (backendMessage) {
      pushBackendMessage(backendMessage);
      continue;
    }

    if (message.role === 'user' && message.id.startsWith('user-')) {
      const matchingBackendUser = takeMatchingBackendUser(message.content);
      merged.push(matchingBackendUser ?? message);
      continue;
    }

    if (
      isLiveStreamingAssistantMessage(message)
      || isLocalTerminalAssistantMessage(message)
    ) {
      merged.push(message);
    }
  }

  for (const message of backendMessages) {
    pushBackendMessage(message);
  }

  return merged;
}
