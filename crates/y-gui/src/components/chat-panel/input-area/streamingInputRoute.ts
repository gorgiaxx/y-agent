export type StreamingInputRoute =
  | { kind: 'steer'; text: string }
  | { kind: 'todo'; text: string };

/** Route active-run input while preserving steer as the default mode. */
export function routeStreamingInput(input: string): StreamingInputRoute {
  const trimmed = input.trim();
  const todoMatch = trimmed.match(/^\/todo(?:\s+([\s\S]*))?$/i);
  if (todoMatch) {
    return { kind: 'todo', text: (todoMatch[1] ?? '').trim() };
  }
  return { kind: 'steer', text: trimmed };
}
