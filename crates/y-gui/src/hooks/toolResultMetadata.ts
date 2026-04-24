import type { ToolResultRecord } from './chatStreamTypes';

function asObject(value: unknown): Record<string, unknown> | null {
  return value != null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function stableStringify(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map((item) => stableStringify(item)).join(',')}]`;
  }
  if (value != null && typeof value === 'object') {
    const entries = Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, item]) => `${JSON.stringify(key)}:${stableStringify(item)}`);
    return `{${entries.join(',')}}`;
  }
  return JSON.stringify(value);
}

function parseResultPreview(resultPreview: string): Record<string, unknown> | null {
  try {
    return asObject(JSON.parse(resultPreview));
  } catch {
    return null;
  }
}

export function parseToolResultStatus(name: string, resultPreview: string): string | null {
  const parsed = parseResultPreview(resultPreview);
  if (!parsed) {
    return null;
  }

  if (typeof parsed.status === 'string') {
    return parsed.status;
  }

  if (name === 'AskUser') {
    const answers = asObject(parsed.answers);
    if (answers) {
      return 'answered';
    }
  }

  return null;
}

function buildAskUserMergeKey(entry: Record<string, unknown>): string {
  const argumentsKey = String(entry.arguments ?? '');
  const resultPreview = String(entry.result_preview ?? '');
  const parsed = parseResultPreview(resultPreview);
  const status = parseToolResultStatus('AskUser', resultPreview) ?? 'unknown';
  const answersKey = parsed ? stableStringify(parsed.answers ?? null) : resultPreview;

  return ['AskUser', argumentsKey, status, answersKey].join('::');
}

function buildToolResultMergeKey(entry: Record<string, unknown>): string {
  const name = String(entry.name ?? '');
  if (name === 'AskUser') {
    return buildAskUserMergeKey(entry);
  }

  const metadata = entry.metadata;
  const metadataKey = metadata == null ? '' : JSON.stringify(metadata);
  return [
    name,
    String(entry.arguments ?? ''),
    String(entry.result_preview ?? ''),
    metadataKey,
  ].join('::');
}

function toToolResultMetadata(records: ToolResultRecord[]): Array<Record<string, unknown>> {
  return records.filter((tr) => tr.state !== 'running').map((tr) => {
    const entry: Record<string, unknown> = {
      name: tr.name,
      arguments: tr.arguments ?? '',
      success: tr.success,
      duration_ms: tr.durationMs,
      result_preview: tr.resultPreview,
    };
    if (tr.urlMeta) {
      try {
        entry.url_meta = JSON.parse(tr.urlMeta) as Record<string, unknown>;
      } catch {
        entry.url_meta = tr.urlMeta;
      }
    }
    if (tr.metadata) {
      entry.metadata = tr.metadata;
    }
    return entry;
  });
}

export function mergeToolResultMetadata(
  backend: unknown,
  streamed: ToolResultRecord[] | undefined,
): Array<Record<string, unknown>> | undefined {
  const backendRecords = Array.isArray(backend)
    ? backend.filter(
      (entry): entry is Record<string, unknown> =>
        entry != null && typeof entry === 'object',
    )
    : [];
  const streamRecords = streamed ? toToolResultMetadata(streamed) : [];

  if (backendRecords.length === 0) {
    return streamRecords.length > 0 ? streamRecords : undefined;
  }
  if (streamRecords.length === 0) {
    return backendRecords;
  }

  const merged: Array<Record<string, unknown>> = [];
  const seen = new Set<string>();

  for (const entry of [...streamRecords, ...backendRecords]) {
    const key = buildToolResultMergeKey(entry);
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(entry);
  }

  return merged;
}
