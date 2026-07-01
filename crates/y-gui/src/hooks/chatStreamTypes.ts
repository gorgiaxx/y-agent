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

const ORCHESTRATOR_AGENT_SUFFIX = '-orchestrator';

// The main chat shows only the GLOBAL view: root-agent output and orchestrator
// summary cards (the `Plan` / `Loop` tool cards, emitted by `plan-orchestrator`
// / `loop-orchestrator`). Sub-agents run under their own child sessions
// (e.g. `plan-writer`, `plan-phase-executor:phase-N`, `loop-executor`, or
// Task-delegated agents) and are surfaced in their own sub-chat, NOT flooded
// into the main chat.
export function shouldDisplayStreamingAgent(
  agentName?: string,
  rootAgentNames: Iterable<string> = [DEFAULT_ROOT_AGENT_NAME],
): boolean {
  if (agentName == null || agentName === '') {
    return true;
  }

  if (agentName.endsWith(ORCHESTRATOR_AGENT_SUFFIX)) {
    return true;
  }

  for (const rootAgentName of rootAgentNames) {
    if (agentName === rootAgentName) {
      return true;
    }
  }

  return false;
}

export function shouldDisplayStreamingContentAgent(
  agentName?: string,
  rootAgentNames: Iterable<string> = [DEFAULT_ROOT_AGENT_NAME],
): boolean {
  return shouldDisplayStreamingAgent(agentName, rootAgentNames);
}

/// Decide whether a progress event belongs to a genuine sub-session (a plan
/// phase or loop round running under its own child session) versus the run's
/// parent session. Task-delegated sub-agents reuse the parent session id, so
/// they are NOT sub-sessions and remain subject to the main-chat agent filter.
export function isSubSessionEvent(
  childSessionId: string | undefined | null,
  parentSessionId: string | undefined | null,
): boolean {
  return !!childSessionId && childSessionId !== parentSessionId;
}
