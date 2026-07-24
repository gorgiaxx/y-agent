import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';

import {
  createSessionInputStates,
  deserializeSessionProviderSelections,
  getSessionInputState,
  serializeSessionProviderSelections,
  setSessionDraft,
  setSessionProvider,
} from '../hooks/sessionInputState';

describe('sessionInputState', () => {
  it('keeps draft content and provider selection isolated by session', () => {
    let state = createSessionInputStates();

    state = setSessionDraft(state, 'session-1', {
      text: 'Question still being written in session 1',
      skills: ['research'],
    });
    state = setSessionProvider(state, 'session-1', 'provider-a');
    state = setSessionDraft(state, 'session-2', {
      text: 'Different unfinished question in session 2',
      skills: [],
    });
    state = setSessionProvider(state, 'session-2', 'provider-b');

    expect(getSessionInputState(state, 'session-1', 'auto')).toEqual({
      draft: {
        text: 'Question still being written in session 1',
        skills: ['research'],
      },
      providerId: 'provider-a',
    });
    expect(getSessionInputState(state, 'session-2', 'auto')).toEqual({
      draft: {
        text: 'Different unfinished question in session 2',
        skills: [],
      },
      providerId: 'provider-b',
    });
  });

  it('uses the current default provider without sharing another session draft', () => {
    const state = setSessionProvider(
      setSessionDraft(createSessionInputStates(), 'session-1', {
        text: 'Session 1 draft',
        skills: [],
      }),
      'session-1',
      'provider-a',
    );

    expect(getSessionInputState(state, 'session-2', 'auto')).toEqual({
      draft: { text: '', skills: [] },
      providerId: 'auto',
    });
  });

  it('persists provider selections per session without persisting unfinished drafts', () => {
    let state = createSessionInputStates();
    state = setSessionDraft(state, 'session-1', {
      text: 'private unfinished draft',
      skills: ['research'],
    });
    state = setSessionProvider(state, 'session-1', 'provider-a');
    state = setSessionProvider(state, 'session-2', 'provider-b');

    const serialized = serializeSessionProviderSelections(state);
    const restored = deserializeSessionProviderSelections(serialized);

    expect(serialized).not.toContain('private unfinished draft');
    expect(getSessionInputState(restored, 'session-1', 'auto')).toEqual({
      draft: { text: '', skills: [] },
      providerId: 'provider-a',
    });
    expect(getSessionInputState(restored, 'session-2', 'auto')).toEqual({
      draft: { text: '', skills: [] },
      providerId: 'provider-b',
    });
  });

  it('falls back to auto when a persisted provider is no longer available', () => {
    const state = setSessionProvider(
      createSessionInputStates(),
      'session-1',
      'removed-provider',
    );

    expect(getSessionInputState(state, 'session-1', 'auto', ['provider-a'])).toEqual({
      draft: { text: '', skills: [] },
      providerId: 'auto',
    });
    expect(getSessionInputState(state, 'session-1', 'auto', ['removed-provider'])).toEqual({
      draft: { text: '', skills: [] },
      providerId: 'removed-provider',
    });
  });

  it('fails closed when persisted provider selections are corrupt or unsupported', () => {
    expect(deserializeSessionProviderSelections('{not-json')).toEqual({});
    expect(deserializeSessionProviderSelections(JSON.stringify({
      version: 2,
      providers: { 'session-1': 'provider-a' },
    }))).toEqual({});
  });

  it('wires the active session state into the chat input and send configuration', () => {
    const chatView = readFileSync(
      new URL('../views/ChatView.tsx', import.meta.url),
      'utf8',
    );
    const inputArea = readFileSync(
      new URL('../components/chat-panel/input-area/InputArea.tsx', import.meta.url),
      'utf8',
    );

    expect(chatView).toContain('const activeInputState = getSessionInputState(');
    expect(chatView).toContain('usePersistentState<SessionInputStates>(');
    expect(chatView).toContain('STORAGE_KEYS.SESSION_INPUT_STATES');
    expect(chatView).toContain('selectedProviderId: activeInputState.providerId');
    expect(chatView).toContain('content: activeInputState.draft');
    expect(chatView).toContain('onContentChange: handleDraftChange');
    expect(inputArea).toContain('contentEditableRef.current?.setContent(content)');
    expect(inputArea).toContain('onContentChange?.(nextContent)');
  });
});
