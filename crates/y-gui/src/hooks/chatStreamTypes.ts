export interface ToolResultRecord {
  name: string;
  /** Serialised tool arguments (JSON string). Available from persisted metadata. */
  arguments?: string;
  success: boolean;
  durationMs: number;
  resultPreview: string;
  /** Compact URL metadata JSON (url, title, favicon_url) for Browser/WebFetch. */
  urlMeta?: string;
  /** Optional structured metadata for presentation layers. */
  metadata?: Record<string, unknown>;
}

export function shouldDisplayStreamingAgent(
  agentName?: string,
  rootAgentNames: Iterable<string> = ['chat-turn'],
): boolean {
  if (agentName == null || agentName === '') {
    return true;
  }

  for (const rootAgentName of rootAgentNames) {
    if (agentName === rootAgentName) {
      return true;
    }
  }

  return false;
}
