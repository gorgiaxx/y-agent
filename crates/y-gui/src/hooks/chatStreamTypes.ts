import { DEFAULT_ROOT_AGENT_NAME } from '../constants/agents';

export interface ToolResultRecord {
  name: string;
  /** Serialised tool arguments (JSON string). Available from persisted metadata. */
  arguments?: string;
  success: boolean;
  durationMs: number;
  resultPreview: string;
  /** Live-only state used before a tool result has arrived. */
  state?: 'running' | 'completed';
  /** Compact URL metadata JSON (url, title, favicon_url) for Browser/WebFetch. */
  urlMeta?: string;
  /** Optional structured metadata for presentation layers. */
  metadata?: Record<string, unknown>;
}

const PLAN_AGENT_PREFIX = 'plan-';

export function shouldDisplayStreamingAgent(
  agentName?: string,
  rootAgentNames: Iterable<string> = [DEFAULT_ROOT_AGENT_NAME],
): boolean {
  if (agentName == null || agentName === '') {
    return true;
  }

  if (agentName.startsWith(PLAN_AGENT_PREFIX)) {
    return true;
  }

  for (const rootAgentName of rootAgentNames) {
    if (agentName === rootAgentName) {
      return true;
    }
  }

  return false;
}
