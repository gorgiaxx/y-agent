import { useEffect, useMemo, useState } from 'react';

import { useChatContext, useSessionsContext } from '../providers/AppContexts';
import { transport } from '../lib';
import type { Message, DiagSubagentCompleted } from '../types';
import type { ToolResultRecord } from './chatStreamTypes';
import {
  canonicalToolName,
  extractFileToolMeta,
  basename,
  extractPlanDisplayMeta,
  extractLoopDisplayMeta,
  type PlanDisplayMeta,
  type LoopDisplayMeta,
} from '../components/chat-panel/chat-box/toolCallUtils';

export interface ModifiedFileEntry {
  filePath: string;
  toolType: 'edit' | 'write';
  displayName: string;
  count: number;
  diffs: Array<{ oldString: string; newString: string }>;
}

export interface ChildSessionSummary {
  id: string;
  title: string | null;
  sessionType: string;
  agentId: string | null;
  messageCount: number;
  createdAt: string;
  /** Last update time (RFC 3339) from the backend; for a finished sub-agent
   *  this is the completion time. Empty string when unavailable. */
  updatedAt: string;
  /** Derived display status: "running" while the parent turn is streaming and
   *  no completion event has been received for this child, otherwise
   *  "completed". */
  status: 'running' | 'completed';
}

export interface UseInfoPanelReturn {
  modifiedFiles: ModifiedFileEntry[];
  plans: PlanDisplayMeta[];
  loopStatus: LoopDisplayMeta | null;
  childSessions: ChildSessionSummary[];
  hasActivity: boolean;
}

function parseMetaToolResults(messages: Message[]): ToolResultRecord[] {
  const results: ToolResultRecord[] = [];
  for (const msg of messages) {
    if (msg.role !== 'assistant') continue;
    const metaResults = msg.metadata?.tool_results;
    if (!Array.isArray(metaResults)) continue;
    for (const tr of metaResults as Array<Record<string, unknown>>) {
      results.push({
        name: String(tr.name ?? ''),
        arguments: String(tr.arguments ?? ''),
        success: Boolean(tr.success),
        durationMs: Number(tr.duration_ms ?? 0),
        resultPreview: String(tr.result_preview ?? ''),
        urlMeta: tr.url_meta != null ? JSON.stringify(tr.url_meta) : undefined,
        metadata: tr.metadata && typeof tr.metadata === 'object'
          ? tr.metadata as Record<string, unknown>
          : undefined,
      });
    }
  }
  return results;
}

export function collectModifiedFilesForInfoPanel(records: ToolResultRecord[]): ModifiedFileEntry[] {
  const map = new Map<string, ModifiedFileEntry>();
  for (const rec of records) {
    const name = canonicalToolName(rec.name);
    if (name !== 'FileEdit' && name !== 'FileWrite') continue;
    const meta = extractFileToolMeta(name, rec.arguments ?? '');
    if (!meta || meta.toolType === 'read') continue;
    const existing = map.get(meta.filePath);
    if (existing) {
      existing.count += 1;
      if (meta.toolType === 'edit' && meta.oldString !== undefined && meta.newString !== undefined) {
        existing.diffs.push({
          oldString: meta.oldString,
          newString: meta.newString,
        });
      }
    } else {
      map.set(meta.filePath, {
        filePath: meta.filePath,
        toolType: meta.toolType as 'edit' | 'write',
        displayName: basename(meta.filePath),
        count: 1,
        diffs: meta.toolType === 'edit' && meta.oldString !== undefined && meta.newString !== undefined
          ? [{ oldString: meta.oldString, newString: meta.newString }]
          : [],
      });
    }
  }
  return Array.from(map.values());
}

function planIdentityKey(plan: PlanDisplayMeta): string {
  if (plan.planFile) return plan.planFile;
  if (plan.kind === 'plan_execution' && plan.planRunId) return plan.planRunId;
  return plan.planTitle || 'plan';
}

/// Collect every distinct plan from the tool-result stream, collapsing each
/// plan's lifecycle (plan_stage -> plan_execution, and any revisions) into a
/// single entry at its most recent state. Order follows first appearance so
/// the list stays stable as plans progress.
export function collectPlansForInfoPanel(records: ToolResultRecord[]): PlanDisplayMeta[] {
  const order: string[] = [];
  const byKey = new Map<string, PlanDisplayMeta>();
  for (const rec of records) {
    if (canonicalToolName(rec.name) !== 'Plan') continue;
    const display = extractPlanDisplayMeta(rec.metadata, rec.resultPreview);
    if (!display) continue;
    const key = planIdentityKey(display);
    if (!byKey.has(key)) order.push(key);
    byKey.set(key, display);
  }
  return order.map((key) => byKey.get(key)!);
}

/// Merge the persisted plan history (from the store) with the live, tool-stream
/// derived plans. Keyed by plan identity: the live entry wins when a plan exists
/// in both (it carries real-time, in-progress granularity), while persisted-only
/// plans (outside the loaded message window) fill in the rest. Persisted order
/// (chronological) is preserved; live-only plans are appended.
export function mergePlans(
  persisted: PlanDisplayMeta[],
  live: PlanDisplayMeta[],
): PlanDisplayMeta[] {
  const order: string[] = [];
  const byKey = new Map<string, PlanDisplayMeta>();
  for (const plan of persisted) {
    const key = planIdentityKey(plan);
    if (!byKey.has(key)) order.push(key);
    byKey.set(key, plan);
  }
  for (const plan of live) {
    const key = planIdentityKey(plan);
    if (!byKey.has(key)) order.push(key);
    byKey.set(key, plan);
  }
  return order.map((key) => byKey.get(key)!);
}

function findLatestLoop(records: ToolResultRecord[]): LoopDisplayMeta | null {
  for (let i = records.length - 1; i >= 0; i--) {
    const rec = records[i];
    if (canonicalToolName(rec.name) !== 'Loop') continue;
    const display = extractLoopDisplayMeta(rec.metadata, rec.resultPreview);
    if (display) return display;
  }
  return null;
}

export function useInfoPanel(): UseInfoPanelReturn {
  const { messages, toolResults, streamingSessionIds } = useChatContext();
  const { activeSessionId } = useSessionsContext();
  const [persistedPlans, setPersistedPlans] = useState<PlanDisplayMeta[]>([]);

  // Refetch persisted history when the active session's turn starts/ends so a
  // plan that completes within the same session refreshes its authoritative
  // status without requiring a session switch.
  const activeStreaming = !!activeSessionId && streamingSessionIds.has(activeSessionId);

  // Load the full persisted plan history for the active session so plans
  // outside the loaded message window (and after restart) remain visible.
  useEffect(() => {
    let cancelled = false;
    const load = async (): Promise<PlanDisplayMeta[]> => {
      if (!activeSessionId) return [];
      try {
        const rows = await transport.invoke<unknown[]>(
          'session_list_plan_runs',
          { sessionId: activeSessionId },
        );
        return (Array.isArray(rows) ? rows : [])
          .map((row) => extractPlanDisplayMeta(row))
          .filter((plan): plan is PlanDisplayMeta => plan != null);
      } catch {
        return [];
      }
    };
    load().then((plans) => {
      if (!cancelled) setPersistedPlans(plans);
    });
    return () => {
      cancelled = true;
    };
  }, [activeSessionId, activeStreaming]);

  // Load the active session's sub-agent child sessions (plan phases, loop
  // rounds, delegated tasks) so they can be opened as drill-in sub-chats.
  // Reloaded when the active session changes, when streaming starts/ends, and
  // when a subagent_completed broadcast arrives for this session (so Task-
  // delegated sub-agents appear without a session switch).
  const [childSessions, setChildSessions] = useState<ChildSessionSummary[]>([]);
  const [childReloadTick, setChildReloadTick] = useState(0);
  // Child session ids that have received a `subagent_completed` event during
  // the current parent turn. Used to mark individual children as completed
  // before the parent turn finishes streaming (so a finished phase shows
  // "completed" while a later phase is still running). Tagged with the
  // session id they belong to so a session switch naturally invalidates the
  // set via the useMemo derivation below (no setState-in-effect cascade).
  const [completedChildren, setCompletedChildren] = useState<{
    sessionId: string;
    ids: Set<string>;
  }>({ sessionId: '', ids: new Set() });
  useEffect(() => {
    let cancelled = false;
    const load = async (): Promise<ChildSessionSummary[]> => {
      if (!activeSessionId) return [];
      try {
        const rows = await transport.invoke<Array<Record<string, unknown>>>(
          'session_list_children',
          { sessionId: activeSessionId },
        );
        return (Array.isArray(rows) ? rows : []).map((r) => ({
          id: String(r.id ?? ''),
          title: typeof r.title === 'string' ? r.title : null,
          sessionType: String(r.session_type ?? ''),
          agentId: typeof r.agent_id === 'string' ? r.agent_id : null,
          messageCount: Number(r.message_count ?? 0),
          createdAt: String(r.created_at ?? ''),
          updatedAt: typeof r.updated_at === 'string' ? r.updated_at : '',
          status: 'completed' as const,
        })).filter((c) => c.id !== '');
      } catch {
        return [];
      }
    };
    load().then((rows) => {
      if (!cancelled) setChildSessions(rows);
    });
    return () => {
      cancelled = true;
    };
  }, [activeSessionId, activeStreaming, childReloadTick]);

  // Listen for subagent_completed broadcasts. The event's `session_id` is the
  // child session UUID for plan phases / loop rounds / Task-delegated agents
  // (emitted by the executor and the plan/loop orchestrators), and the parent
  // session UUID for the duplicate event emitted by DiagnosticsAgentDelegator.
  // We mark a known child as completed on a child-UUID match, and always bump
  // the reload tick (on either match) so the backend's authoritative
  // `updated_at` completion time is fetched.
  useEffect(() => {
    if (!activeSessionId) return;
    const unlisten = transport.listen<DiagSubagentCompleted>(
      'diagnostics:event',
      (event) => {
        const ev = event.payload;
        if (ev.type !== 'subagent_completed') return;
        const sid = ev.session_id;
        if (!sid) return;
        // Mark the child as completed when the event carries its UUID
        // directly (plan phase / loop round / Task-delegated agent). The
        // duplicate event from DiagnosticsAgentDelegator carries the parent
        // UUID and is handled by the reload below.
        setChildSessions((prev) => {
          if (!prev.some((c) => c.id === sid)) return prev;
          setCompletedChildren((prev) =>
            prev.sessionId === activeSessionId && prev.ids.has(sid)
              ? prev
              : {
                  sessionId: activeSessionId,
                  ids: new Set(
                    prev.sessionId === activeSessionId ? prev.ids : [],
                  ).add(sid),
                },
          );
          return prev;
        });
        // Always reload to fetch the authoritative `updated_at` completion
        // timestamp from the backend.
        setChildReloadTick((t) => t + 1);
      },
    );
    return () => {
      unlisten.then((fn) => fn?.()).catch(() => {});
    };
  }, [activeSessionId]);

  return useMemo(() => {
    const historical = parseMetaToolResults(messages);
    const allRecords = [...historical, ...toolResults];

    const modifiedFiles = collectModifiedFilesForInfoPanel(allRecords);
    const livePlans = collectPlansForInfoPanel(allRecords);
    const plans = mergePlans(persistedPlans, livePlans);
    const loopStatus = findLatestLoop(allRecords);

    // Derive per-child status: a child is "running" only while the parent
    // turn is streaming AND no completion event has arrived for it yet.
    // Once the parent turn ends, all children are considered completed.
    // The completed-ids set is only valid for the active session; a session
    // switch naturally yields an empty set here.
    const validCompletedIds = completedChildren.sessionId === activeSessionId
      ? completedChildren.ids
      : new Set<string>();
    const childrenWithStatus = activeStreaming
      ? childSessions.map((c) =>
          validCompletedIds.has(c.id)
            ? { ...c, status: 'completed' as const }
            : { ...c, status: 'running' as const },
        )
      : childSessions.map((c) => ({ ...c, status: 'completed' as const }));

    return {
      modifiedFiles,
      plans,
      loopStatus,
      childSessions: childrenWithStatus,
      hasActivity:
        modifiedFiles.length > 0
        || plans.length > 0
        || loopStatus !== null
        || childrenWithStatus.length > 0,
    };
  }, [messages, toolResults, persistedPlans, childSessions, activeStreaming, completedChildren, activeSessionId]);
}
