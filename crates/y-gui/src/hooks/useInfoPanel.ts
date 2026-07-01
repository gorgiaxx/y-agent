import { useEffect, useMemo, useState } from 'react';

import { useChatContext, useSessionsContext } from '../providers/AppContexts';
import { transport } from '../lib';
import type { Message } from '../types';
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
  const [childSessions, setChildSessions] = useState<ChildSessionSummary[]>([]);
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
  }, [activeSessionId, activeStreaming]);

  return useMemo(() => {
    const historical = parseMetaToolResults(messages);
    const allRecords = [...historical, ...toolResults];

    const modifiedFiles = collectModifiedFilesForInfoPanel(allRecords);
    const livePlans = collectPlansForInfoPanel(allRecords);
    const plans = mergePlans(persistedPlans, livePlans);
    const loopStatus = findLatestLoop(allRecords);

    return {
      modifiedFiles,
      plans,
      loopStatus,
      childSessions,
      hasActivity:
        modifiedFiles.length > 0
        || plans.length > 0
        || loopStatus !== null
        || childSessions.length > 0,
    };
  }, [messages, toolResults, persistedPlans, childSessions]);
}
