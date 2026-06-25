import { describe, expect, it } from 'vitest';

import type { Message } from '../types';
import {
  isSteerMessage,
  steerRunEnd,
  mergeSteeredTurn,
} from '../components/chat-panel/steerCoalescing';

function user(id: string, content = 'hi'): Message {
  return { id, role: 'user', content, timestamp: '', tool_calls: [] };
}

function steer(id: string, content = 'steered', steerId = id): Message {
  return {
    id,
    role: 'user',
    content,
    timestamp: '',
    tool_calls: [],
    metadata: { kind: 'steer', steer_id: steerId },
  };
}

function assistant(id: string, meta: Record<string, unknown> = {}, content = ''): Message {
  return { id, role: 'assistant', content, timestamp: '', tool_calls: [], metadata: meta };
}

describe('isSteerMessage', () => {
  it('detects tagged steer user messages and ignores normal ones', () => {
    expect(isSteerMessage(steer('s1'))).toBe(true);
    expect(isSteerMessage(user('u1'))).toBe(false);
    expect(isSteerMessage(assistant('a1'))).toBe(false);
  });
});

describe('steerRunEnd', () => {
  it('absorbs an assistant -> steer -> assistant run', () => {
    const msgs = [user('u1'), assistant('a1'), steer('s1'), assistant('a2'), user('u2')];
    expect(steerRunEnd(msgs, 1)).toEqual({ end: 3, sawSteer: true });
  });

  it('absorbs multiple steers at the same boundary', () => {
    const msgs = [assistant('a1'), steer('s1'), steer('s2'), assistant('a2')];
    expect(steerRunEnd(msgs, 0)).toEqual({ end: 3, sawSteer: true });
  });

  it('handles a leading steer', () => {
    const msgs = [steer('s1'), assistant('a2'), user('u2')];
    expect(steerRunEnd(msgs, 0)).toEqual({ end: 1, sawSteer: true });
  });

  it('does NOT merge two adjacent assistants without a steer between them', () => {
    const msgs = [assistant('a1'), assistant('a2')];
    expect(steerRunEnd(msgs, 0)).toEqual({ end: 0, sawSteer: false });
  });

  it('stops at a real user message', () => {
    const msgs = [assistant('a1'), steer('s1'), assistant('a2'), user('u2'), assistant('a3')];
    expect(steerRunEnd(msgs, 0)).toEqual({ end: 2, sawSteer: true });
  });
});

describe('mergeSteeredTurn', () => {
  it('concatenates iteration data and anchors steers at the combined boundary', () => {
    const a1 = assistant(
      'a1',
      {
        iteration_texts: ['look\n'],
        iteration_tool_counts: [1],
        tool_results: [{ name: 'Read' }],
      },
      'look\n',
    );
    const a2 = assistant(
      'a2',
      {
        iteration_texts: ['search\n'],
        iteration_tool_counts: [1],
        tool_results: [{ name: 'Grep' }],
        final_response: 'done',
        input_tokens: 10,
      },
      'search\n',
    );
    const merged = mergeSteeredTurn([a1, steer('s1', 'focus'), a2]);

    expect(merged.id).toBe('a2');
    expect(merged.role).toBe('assistant');
    expect(merged.content).toBe('look\nsearch\n');
    expect(merged.metadata?.iteration_texts).toEqual(['look\n', 'search\n']);
    expect(merged.metadata?.iteration_tool_counts).toEqual([1, 1]);
    expect(merged.metadata?.tool_results).toEqual([{ name: 'Read' }, { name: 'Grep' }]);
    expect(merged.metadata?.final_response).toBe('done');
    expect(merged.metadata?.injected_steers).toEqual([
      { after_iteration: 1, text: 'focus', steer_id: 's1' },
    ]);
  });

  it('records a leading steer at boundary 0', () => {
    const a1 = assistant('a1', { iteration_texts: [], final_response: 'final' });
    const merged = mergeSteeredTurn([steer('s1', 'wait'), a1]);
    expect(merged.metadata?.injected_steers).toEqual([
      { after_iteration: 0, text: 'wait', steer_id: 's1' },
    ]);
  });
});
