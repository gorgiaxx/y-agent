import { useMemo } from 'react';

import { useChatContext } from '../providers/AppContexts';
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

export interface UseInfoPanelReturn {
  modifiedFiles: ModifiedFileEntry[];
  planStatus: PlanDisplayMeta | null;
  loopStatus: LoopDisplayMeta | null;
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

function findLatestPlan(records: ToolResultRecord[]): PlanDisplayMeta | null {
  for (let i = records.length - 1; i >= 0; i--) {
    const rec = records[i];
    if (canonicalToolName(rec.name) !== 'Plan') continue;
    const display = extractPlanDisplayMeta(rec.metadata, rec.resultPreview);
    if (display) return display;
  }
  return null;
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
  const { messages, toolResults } = useChatContext();

  return useMemo(() => {
    const historical = parseMetaToolResults(messages);
    const allRecords = [...historical, ...toolResults];

    const modifiedFiles = collectModifiedFilesForInfoPanel(allRecords);
    const planStatus = findLatestPlan(allRecords);
    const loopStatus = findLatestLoop(allRecords);

    return {
      modifiedFiles,
      planStatus,
      loopStatus,
      hasActivity: modifiedFiles.length > 0 || planStatus !== null || loopStatus !== null,
    };
  }, [messages, toolResults]);
}
