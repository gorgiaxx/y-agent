import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { AgentSessionRail } from '../components/agents/AgentSessionRail';

describe('AgentSessionRail', () => {
  const baseProps = {
    activeSessionId: null,
    onEdit: () => {},
    onNewSession: () => {},
    onSelectSession: () => {},
    onDeleteSession: () => {},
  };

  it('shows a loading state while agent sessions are being fetched', () => {
    const html = renderToStaticMarkup(
      <AgentSessionRail
        {...baseProps}
        sessions={[]}
        loading
        streamingSessionIds={new Set()}
      />,
    );

    expect(html).toContain('Loading sessions...');
  });

  it('marks streaming sessions with the same activity affordance as the main chat list', () => {
    const html = renderToStaticMarkup(
      <AgentSessionRail
        {...baseProps}
        sessions={[
          {
            id: 'session-1',
            title: 'Agent Debug',
            created_at: '2026-04-15T09:55:00Z',
            message_count: 3,
            updated_at: '2026-04-15T10:00:00Z',
          },
        ]}
        loading={false}
        streamingSessionIds={new Set(['session-1'])}
      />,
    );

    expect(html).toContain('session-item--streaming');
    expect(html).toContain('now');
  });
});
