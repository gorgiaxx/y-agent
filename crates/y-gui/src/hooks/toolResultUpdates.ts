import type { InterleavedSegment } from './useInterleavedSegments';
import type { ToolResultRecord } from './chatStreamTypes';

interface ToolResultRecordUpdate {
  records: ToolResultRecord[];
  replacedIndex: number | null;
}

interface ToolResultSegmentUpdate {
  segments: InterleavedSegment[];
  replacedIndex: number | null;
}

function parseStatus(resultPreview: string): string | null {
  try {
    const parsed = JSON.parse(resultPreview) as Record<string, unknown>;
    return typeof parsed.status === 'string' ? parsed.status : null;
  } catch {
    return null;
  }
}

function shouldReplacePendingAskUser(
  current: ToolResultRecord,
  next: ToolResultRecord,
): boolean {
  if (current.name !== 'AskUser' || next.name !== 'AskUser') return false;
  if ((current.arguments ?? '') !== (next.arguments ?? '')) return false;

  const currentStatus = parseStatus(current.resultPreview);
  const nextStatus = parseStatus(next.resultPreview);

  return currentStatus === 'pending' && nextStatus !== null && nextStatus !== 'pending';
}

function asObject(value: unknown): Record<string, unknown> | null {
  return value != null && typeof value === 'object'
    ? value as Record<string, unknown>
    : null;
}

function extractPlanProgressKey(record: ToolResultRecord): string | null {
  if (record.name !== 'Plan') return null;

  const meta = asObject(record.metadata);
  const display = asObject(meta?.display);
  if (!display) return null;

  const kind = typeof display.kind === 'string' ? display.kind : '';
  if (kind !== 'plan_execution' && kind !== 'plan_stage') return null;

  const planFile = typeof display.plan_file === 'string' ? display.plan_file : '';
  const planTitle = typeof display.plan_title === 'string' ? display.plan_title : '';
  const fallbackArgs = record.arguments ?? '';

  const key = planFile || planTitle || fallbackArgs;
  return key || null;
}

function shouldReplacePlanProgress(
  current: ToolResultRecord,
  next: ToolResultRecord,
): boolean {
  const currentKey = extractPlanProgressKey(current);
  const nextKey = extractPlanProgressKey(next);

  return currentKey != null && nextKey != null && currentKey === nextKey;
}

function findPendingAskUserIndex(
  records: ToolResultRecord[],
  next: ToolResultRecord,
): number {
  for (let i = records.length - 1; i >= 0; i--) {
    if (shouldReplacePendingAskUser(records[i], next)) {
      return i;
    }
  }
  return -1;
}

function findPlanProgressIndex(
  records: ToolResultRecord[],
  next: ToolResultRecord,
): number {
  for (let i = records.length - 1; i >= 0; i--) {
    if (shouldReplacePlanProgress(records[i], next)) {
      return i;
    }
  }
  return -1;
}

export function upsertToolResultRecord(
  records: ToolResultRecord[],
  next: ToolResultRecord,
): ToolResultRecordUpdate {
  const replaceIdx = findPendingAskUserIndex(records, next);
  if (replaceIdx >= 0) {
    const updated = [...records];
    updated[replaceIdx] = next;
    return { records: updated, replacedIndex: replaceIdx };
  }

  const replacePlanIdx = findPlanProgressIndex(records, next);
  if (replacePlanIdx >= 0) {
    const updated = [...records];
    updated[replacePlanIdx] = next;
    return { records: updated, replacedIndex: replacePlanIdx };
  }

  return {
    records: [...records, next],
    replacedIndex: null,
  };
}

export function upsertToolResultSegment(
  segments: InterleavedSegment[],
  next: ToolResultRecord,
): ToolResultSegmentUpdate {
  const toolSegments = segments
    .map((segment, index) => ({ segment, index }))
    .filter(
      (
        entry,
      ): entry is { segment: Extract<InterleavedSegment, { type: 'tool_result' }>; index: number } =>
        entry.segment.type === 'tool_result',
    );

  for (let i = toolSegments.length - 1; i >= 0; i--) {
    const { segment, index } = toolSegments[i];
    if (shouldReplacePendingAskUser(segment.record, next)) {
      const updated = [...segments];
      updated[index] = { type: 'tool_result', record: next };
      return { segments: updated, replacedIndex: index };
    }
  }

  for (let i = toolSegments.length - 1; i >= 0; i--) {
    const { segment, index } = toolSegments[i];
    if (shouldReplacePlanProgress(segment.record, next)) {
      const updated = [...segments];
      updated[index] = { type: 'tool_result', record: next };
      return { segments: updated, replacedIndex: index };
    }
  }

  return {
    segments: [...segments, { type: 'tool_result', record: next }],
    replacedIndex: null,
  };
}
