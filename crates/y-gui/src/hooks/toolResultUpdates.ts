import type { InterleavedSegment } from './useInterleavedSegments';
import type { ToolResultRecord } from './chatStreamTypes';
import { parseToolResultStatus } from './toolResultMetadata';

interface ToolResultRecordUpdate {
  records: ToolResultRecord[];
  replacedIndex: number | null;
}

interface ToolResultSegmentUpdate {
  segments: InterleavedSegment[];
  replacedIndex: number | null;
}

function shouldReplacePendingAskUser(
  current: ToolResultRecord,
  next: ToolResultRecord,
): boolean {
  if (current.name !== 'AskUser' || next.name !== 'AskUser') return false;
  if ((current.arguments ?? '') !== (next.arguments ?? '')) return false;

  const currentStatus = parseToolResultStatus(current.name, current.resultPreview);
  const nextStatus = parseToolResultStatus(next.name, next.resultPreview);

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

function extractPlanDisplay(record: ToolResultRecord): Record<string, unknown> | null {
  if (record.name !== 'Plan') return null;

  const meta = asObject(record.metadata);
  const display = asObject(meta?.display);
  if (!display) return null;

  const kind = typeof display.kind === 'string' ? display.kind : '';
  if (kind !== 'plan_execution' && kind !== 'plan_stage') return null;

  return display;
}

function shouldReplacePlanProgress(
  current: ToolResultRecord,
  next: ToolResultRecord,
): boolean {
  const currentKey = extractPlanProgressKey(current);
  const nextKey = extractPlanProgressKey(next);

  return currentKey != null && nextKey != null && currentKey === nextKey;
}

function shouldReplacePlanTerminalError(
  records: ToolResultRecord[],
  currentIndex: number,
  next: ToolResultRecord,
): boolean {
  const current = records[currentIndex];
  const matchingRunningIndex = findRunningToolIndex(records, next);

  if (current.name !== 'Plan' || next.name !== 'Plan' || next.success !== false) return false;
  if (extractPlanDisplay(current) == null || extractPlanDisplay(next) != null) return false;

  if (matchingRunningIndex >= 0 && currentIndex > matchingRunningIndex) return true;

  const currentDisplay = extractPlanDisplay(current);
  if (
    matchingRunningIndex < 0
    && currentDisplay != null
    && typeof currentDisplay.stage_status === 'string'
    && currentDisplay.stage_status === 'running'
  ) {
    return true;
  }

  return false;
}

function withPreservedFailedPlanMetadata(
  current: ToolResultRecord,
  next: ToolResultRecord,
): ToolResultRecord {
  if (
    current.name !== 'Plan'
    || next.name !== 'Plan'
    || next.success !== false
    || extractPlanDisplay(current) == null
    || extractPlanDisplay(next) != null
  ) {
    return next;
  }

  const currentMeta = asObject(current.metadata) ?? {};
  const currentDisplay = extractPlanDisplay(current);
  const nextMeta = asObject(next.metadata) ?? {};
  const nextDisplay = currentDisplay
    ? {
        ...currentDisplay,
        ...(currentDisplay.kind === 'plan_stage'
          ? { stage_status: 'failed' }
          : {}),
      }
    : undefined;

  return {
    ...next,
    metadata: {
      ...currentMeta,
      ...nextMeta,
      ...(nextDisplay ? { display: nextDisplay } : {}),
    },
  };
}

function shouldReplaceRunningTool(
  current: ToolResultRecord,
  next: ToolResultRecord,
): boolean {
  if (current.state !== 'running' || current.name !== next.name) return false;
  if (current.name === 'Plan' && !extractPlanDisplay(next)) return true;
  return (current.arguments ?? '') === (next.arguments ?? '');
}

function findRunningToolIndex(
  records: ToolResultRecord[],
  next: ToolResultRecord,
): number {
  for (let i = records.length - 1; i >= 0; i--) {
    if (shouldReplaceRunningTool(records[i], next)) {
      return i;
    }
  }
  return -1;
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

function findPlanTerminalErrorIndex(
  records: ToolResultRecord[],
  next: ToolResultRecord,
): number {
  for (let i = records.length - 1; i >= 0; i--) {
    if (shouldReplacePlanTerminalError(records, i, next)) {
      return i;
    }
  }
  return -1;
}

function replaceRecordAndDropRunningPlanPlaceholder(
  records: ToolResultRecord[],
  replaceIndex: number,
  replacement: ToolResultRecord,
): ToolResultRecordUpdate {
  const updated: ToolResultRecord[] = [];
  let replacedIndex: number | null = null;

  for (let i = 0; i < records.length; i++) {
    const record = records[i];

    if (i === replaceIndex) {
      replacedIndex = updated.length;
      updated.push(replacement);
      continue;
    }

    if (shouldReplaceRunningTool(record, replacement)) {
      continue;
    }

    updated.push(record);
  }

  return { records: updated, replacedIndex };
}

export function upsertToolResultRecord(
  records: ToolResultRecord[],
  next: ToolResultRecord,
): ToolResultRecordUpdate {
  const replacePlanIdx = findPlanProgressIndex(records, next);
  if (replacePlanIdx >= 0) {
    return replaceRecordAndDropRunningPlanPlaceholder(
      records,
      replacePlanIdx,
      next,
    );
  }

  const replacePlanErrorIdx = findPlanTerminalErrorIndex(records, next);
  if (replacePlanErrorIdx >= 0) {
    return replaceRecordAndDropRunningPlanPlaceholder(
      records,
      replacePlanErrorIdx,
      withPreservedFailedPlanMetadata(records[replacePlanErrorIdx], next),
    );
  }

  const runningIdx = findRunningToolIndex(records, next);
  if (runningIdx >= 0) {
    const updated = [...records];
    updated[runningIdx] = next;
    return { records: updated, replacedIndex: runningIdx };
  }

  const replaceIdx = findPendingAskUserIndex(records, next);
  if (replaceIdx >= 0) {
    const updated = [...records];
    updated[replaceIdx] = next;
    return { records: updated, replacedIndex: replaceIdx };
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
  const toolRecords = toolSegments.map((entry) => entry.segment.record);

  const replaceSegmentAndDropRunningPlanPlaceholder = (
    replaceIndex: number,
    replacement: ToolResultRecord,
  ): ToolResultSegmentUpdate => {
    const updated: InterleavedSegment[] = [];
    let replacedIndex: number | null = null;

    for (let i = 0; i < segments.length; i++) {
      const segment = segments[i];

      if (i === replaceIndex) {
        replacedIndex = updated.length;
        updated.push({ type: 'tool_result', record: replacement });
        continue;
      }

      if (
        segment.type === 'tool_result'
        && shouldReplaceRunningTool(segment.record, replacement)
      ) {
        continue;
      }

      updated.push(segment);
    }

    return { segments: updated, replacedIndex };
  };

  for (let i = toolSegments.length - 1; i >= 0; i--) {
    const { segment, index } = toolSegments[i];
    if (shouldReplacePlanProgress(segment.record, next)) {
      return replaceSegmentAndDropRunningPlanPlaceholder(index, next);
    }
  }

  for (let i = toolSegments.length - 1; i >= 0; i--) {
    const { segment, index } = toolSegments[i];
    if (shouldReplacePlanTerminalError(toolRecords, i, next)) {
      return replaceSegmentAndDropRunningPlanPlaceholder(
        index,
        withPreservedFailedPlanMetadata(segment.record, next),
      );
    }
  }

  for (let i = toolSegments.length - 1; i >= 0; i--) {
    const { segment, index } = toolSegments[i];
    if (shouldReplaceRunningTool(segment.record, next)) {
      const updated = [...segments];
      updated[index] = { type: 'tool_result', record: next };
      return { segments: updated, replacedIndex: index };
    }
  }

  for (let i = toolSegments.length - 1; i >= 0; i--) {
    const { segment, index } = toolSegments[i];
    if (shouldReplacePendingAskUser(segment.record, next)) {
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
