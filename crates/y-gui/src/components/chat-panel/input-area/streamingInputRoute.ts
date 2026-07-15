export type StreamingInputRoute =
  | { kind: 'message'; text: string }
  | { kind: 'todo'; text: string };

/** Strip an explicit `/todo` prefix while preserving ordinary message input. */
export function routeStreamingInput(input: string): StreamingInputRoute {
  const trimmed = input.trim();
  const todoMatch = trimmed.match(/^\/todo(?:\s+([\s\S]*))?$/i);
  if (todoMatch) {
    return { kind: 'todo', text: (todoMatch[1] ?? '').trim() };
  }
  return { kind: 'message', text: trimmed };
}
