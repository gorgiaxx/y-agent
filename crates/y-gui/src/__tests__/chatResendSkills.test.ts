// Regression coverage for skill propagation on resend / edit-and-resend.
//
// The backend persists a user message's skills and surfaces them as a
// top-level `skills` field. When a resend (or edit) falls back to a fresh
// `chat_send` -- rather than the checkpoint-based `chat_resend` that recovers
// skills server-side -- the frontend must forward those skills explicitly,
// otherwise both the bubble tag and the backend turn lose their skill context.
//
// `useChatOperations` only depends on `useRef` / `useCallback`, so we stub
// those to pass-throughs and call the factory directly (aliased so it is not
// treated as a hook call) instead of standing up a React renderer.

import { describe, expect, it, vi, beforeEach } from 'vitest';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock('../lib', () => ({
  transport: { invoke: invokeMock },
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() },
}));

vi.mock('react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react')>();
  return {
    ...actual,
    useRef: <T,>(initial: T) => ({ current: initial }),
    useCallback: <T,>(fn: T) => fn,
  };
});

import {
  useChatOperations as createChatOperations,
  type UseChatOperationsReturn,
} from '../hooks/useChatOperations';
import type { ChatSharedRefs } from '../hooks/chatSharedState';
import type { PendingEdit } from '../hooks/useChat';
import type { Message } from '../types';

function makeRefs(): ChatSharedRefs {
  const refs: ChatSharedRefs = {
    activeSessionIdRef: { current: null },
    sessionMessagesRef: { current: new Map<string, Message[]>() },
    sessionActivityRef: { current: new Map() },
    opStatusMapRef: { current: new Map() },
    opStatusRef: { current: 'idle' },
    toolResultsRef: { current: new Map() },
    streamSegsRef: { current: new Map() },
    contextResetMapRef: { current: new Map() },
    compactMapRef: { current: new Map() },
    rootAgentNamesRef: { current: [] },
  };
  return refs;
}

function makeOps(
  refs: ChatSharedRefs,
  pendingEdit: PendingEdit | null = null,
): UseChatOperationsReturn {
  return createChatOperations(
    refs,
    vi.fn(), // setOp
    vi.fn(), // setError
    vi.fn(), // syncVisible
    vi.fn(async () => {}), // loadMessages
    vi.fn(), // invalidateStaleContextResets
    vi.fn(), // markSessionActivity
    pendingEdit,
    vi.fn(), // setPendingEdit
    vi.fn(), // setStreamingSessionIds
    () => 'text_chat', // getRequestModeFromMessage
  );
}

function lastChatSendArgs(): Record<string, unknown> | undefined {
  const calls = invokeMock.mock.calls.filter((c) => c[0] === 'chat_send');
  return calls.length ? (calls[calls.length - 1][1] as Record<string, unknown>) : undefined;
}

function userMessage(overrides: Partial<Message>): Message {
  return {
    id: 'u1',
    role: 'user',
    content: 'hi',
    timestamp: '2026-01-01T00:00:00.000Z',
    tool_calls: [],
    ...overrides,
  };
}

describe('resendLastTurn skill propagation (no-checkpoint fallback)', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('forwards the resent message skills into the fallback chat_send and optimistic bubble', async () => {
    const freshMsgs: Message[] = [
      userMessage({ id: 'u1', content: 'hi', skills: ['drozer-usage-en'] }),
      userMessage({ id: 'a1', role: 'assistant', content: 'yo' }),
    ];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'session_get_messages') return Promise.resolve(freshMsgs);
      if (cmd === 'chat_find_checkpoint_for_resend') return Promise.resolve(null);
      if (cmd === 'chat_send') return Promise.resolve({ runId: 'run-1' });
      return Promise.resolve(undefined);
    });

    const refs = makeRefs();
    const ops = makeOps(refs);
    await ops.resendLastTurn('s1', 'u1', 'hi');

    expect(lastChatSendArgs()?.skills).toEqual(['drozer-usage-en']);

    const optimistic = refs.sessionMessagesRef.current
      .get('s1')
      ?.find((m) => m.role === 'user');
    expect(optimistic?.skills).toEqual(['drozer-usage-en']);
  });

  it('sends null skills when the resent message had none', async () => {
    const freshMsgs: Message[] = [userMessage({ id: 'u1', content: 'hi' })];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'session_get_messages') return Promise.resolve(freshMsgs);
      if (cmd === 'chat_find_checkpoint_for_resend') return Promise.resolve(null);
      if (cmd === 'chat_send') return Promise.resolve({ runId: 'run-1' });
      return Promise.resolve(undefined);
    });

    const refs = makeRefs();
    const ops = makeOps(refs);
    await ops.resendLastTurn('s1', 'u1', 'hi');

    expect(lastChatSendArgs()?.skills).toBeNull();
  });
});

describe('editAndResend skill propagation', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('carries the edited message skills into the resent turn (fallback)', async () => {
    const refs = makeRefs();
    refs.sessionMessagesRef.current.set('s1', [
      userMessage({ id: 'u1', content: 'old', skills: ['drozer-usage-en'] }),
    ]);
    const freshMsgs: Message[] = [
      userMessage({ id: 'u1', content: 'old', skills: ['drozer-usage-en'] }),
    ];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'session_get_messages') return Promise.resolve(freshMsgs);
      if (cmd === 'chat_find_checkpoint_for_resend') return Promise.resolve(null);
      if (cmd === 'chat_send') return Promise.resolve({ runId: 'run-1' });
      return Promise.resolve(undefined);
    });

    const ops = makeOps(refs, { messageId: 'u1', content: 'old', sessionId: 's1' });
    await ops.editAndResend('s1', 'new text');

    const args = lastChatSendArgs();
    expect(args?.message).toBe('new text');
    expect(args?.skills).toEqual(['drozer-usage-en']);
  });
});
