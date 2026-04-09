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

export function shouldDisplayStreamingAgent(agentName?: string): boolean {
  return agentName == null || agentName === '' || agentName === 'chat-turn';
}
