import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { ChatPanel } from '../components/chat-panel/ChatPanel';
import { removeContextResetPoint } from '../hooks/contextResetState';

describe('context reset undo', () => {
  it('removes the selected reset and persists the latest remaining boundary', () => {
    expect(removeContextResetPoint([3, 7, 10], 1)).toEqual({
      points: [3, 10],
      persistedIndex: 10,
    });
    expect(removeContextResetPoint([3], 0)).toEqual({
      points: [],
      persistedIndex: null,
    });
  });

  it('renders an undo button on the context reset divider', () => {
    const html = renderToStaticMarkup(
      <ChatPanel
        messages={[{
          id: 'message-1',
          role: 'user',
          content: 'Existing context',
          timestamp: '2026-07-15T00:00:00.000Z',
          tool_calls: [],
        }]}
        isStreaming={false}
        isLoading={false}
        error={null}
        contextResetPoints={[1]}
        onUndoContextReset={() => {}}
      />,
    );

    expect(html).toContain('aria-label="Undo context reset"');
    expect(html).toContain('>Undo<');
  });

  it('wires the chat session undo handler into the main and agent chat panels', () => {
    const chatView = readFileSync(
      new URL('../views/ChatView.tsx', import.meta.url),
      'utf8',
    );
    const agentStudio = readFileSync(
      new URL('../components/agents/AgentStudio.tsx', import.meta.url),
      'utf8',
    );

    expect(chatView).toContain('onUndoContextReset={chatHooks.removeContextReset}');
    expect(agentStudio).toContain('onUndoContextReset={onUndoContextReset}');
  });
});
