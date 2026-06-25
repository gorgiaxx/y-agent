import type { Message } from '../../types';

/** True when a message is a user steering message (tagged by the backend). */
export function isSteerMessage(msg: Message): boolean {
  return msg.role === 'user' && msg.metadata?.kind === 'steer';
}

/**
 * Find the inclusive end index of a steered-turn run starting at `start`
 * (an assistant or steer message).
 *
 * A run absorbs following steer-user messages and the assistant segments they
 * connect. Two assistant messages that are NOT separated by a steer are never
 * merged (so adjacent assistants from different turns or error/retry partials
 * stay separate). `sawSteer` is false for a lone assistant -- the common
 * non-steered turn, which is rendered as a single message unchanged.
 */
export function steerRunEnd(
  messages: Message[],
  start: number,
): { end: number; sawSteer: boolean } {
  let end = start;
  let sawSteer = isSteerMessage(messages[start]);
  let lastWasAssistant = messages[start].role === 'assistant';

  for (let j = start + 1; j < messages.length; j++) {
    const m = messages[j];
    if (isSteerMessage(m)) {
      sawSteer = true;
      lastWasAssistant = false;
      end = j;
      continue;
    }
    if (m.role === 'assistant') {
      if (lastWasAssistant) break; // no steer between -> different turn / retry
      lastWasAssistant = true;
      end = j;
      continue;
    }
    break; // real user message or other boundary
  }

  return { end, sawSteer };
}

function asArray<T>(value: unknown): T[] {
  return Array.isArray(value) ? (value as T[]) : [];
}

function alignTo<T>(values: T[], length: number, fill: T): T[] {
  if (values.length === length) return values;
  const out = values.slice(0, length);
  while (out.length < length) out.push(fill);
  return out;
}

/**
 * Merge a steered-turn run (`[assistant, steer-user, assistant, ...]`) into a
 * single synthetic assistant message. Iteration/tool arrays are concatenated
 * across assistant segments; turn-level fields (final_response, tokens, cost,
 * reasoning, generated images, stream_error) come from the last assistant
 * segment. Each steer is recorded in `metadata.injected_steers` anchored at the
 * combined iteration boundary, so `buildHistorySegments` can splice an inline
 * steer chip at the true injection position.
 */
export function mergeSteeredTurn(run: Message[]): Message {
  const iterationTexts: string[] = [];
  const iterationReasonings: (string | null)[] = [];
  const iterationDurations: (number | null)[] = [];
  const iterationToolCounts: number[] = [];
  const toolResults: unknown[] = [];
  const injectedSteers: { after_iteration: number; text: string; steer_id?: string }[] = [];
  const contents: string[] = [];
  let lastAssistant: Message | undefined;

  for (const m of run) {
    if (isSteerMessage(m)) {
      const steerId = m.metadata?.steer_id;
      injectedSteers.push({
        after_iteration: iterationTexts.length,
        text: m.content,
        steer_id: typeof steerId === 'string' ? steerId : undefined,
      });
      continue;
    }

    lastAssistant = m;
    contents.push(m.content);
    const meta = m.metadata ?? {};
    const its = asArray<string>(meta.iteration_texts);
    const n = its.length;
    iterationTexts.push(...its);
    iterationReasonings.push(...alignTo(asArray<string | null>(meta.iteration_reasonings), n, null));
    iterationDurations.push(
      ...alignTo(asArray<number | null>(meta.iteration_reasoning_durations_ms), n, null),
    );
    iterationToolCounts.push(...alignTo(asArray<number>(meta.iteration_tool_counts), n, 0));
    toolResults.push(...asArray<unknown>(meta.tool_results));
  }

  const base = lastAssistant ?? run[run.length - 1];
  const mergedMeta: Record<string, unknown> = {
    ...(base.metadata ?? {}),
    iteration_texts: iterationTexts,
    iteration_reasonings: iterationReasonings,
    iteration_reasoning_durations_ms: iterationDurations,
    iteration_tool_counts: iterationToolCounts,
    tool_results: toolResults,
    injected_steers: injectedSteers,
  };

  return {
    ...base,
    role: 'assistant',
    content: contents.join(''),
    metadata: mergedMeta,
  };
}
