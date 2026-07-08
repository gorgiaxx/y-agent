import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { SessionItem } from '../components/shared/SessionItem';
import type { SessionInfo } from '../types';

const session: SessionInfo = {
  id: 's1',
  title: 'Test session',
  created_at: '2026-07-08T00:00:00.000Z',
  updated_at: '2026-07-08T00:00:00.000Z',
  message_count: 3,
};

describe('SessionItem pin button during streaming', () => {
  it('renders the pin button while streaming so the session can be pinned mid-run', () => {
    const html = renderToStaticMarkup(
      <SessionItem
        session={session}
        isActive={false}
        isStreaming={true}
        onPinToggle={() => {}}
        onClick={() => {}}
      />,
    );

    expect(html).toContain('session-item-pin');
    expect(html).toContain('Pin session');
  });

  it('renders the activity indicator alongside the pin button while streaming', () => {
    const html = renderToStaticMarkup(
      <SessionItem
        session={session}
        isActive={false}
        isStreaming={true}
        onPinToggle={() => {}}
        onClick={() => {}}
      />,
    );

    expect(html).toContain('session-item-activity');
    expect(html).toContain('session-item-pin');
  });

  it('renders the pinned pin button while streaming', () => {
    const html = renderToStaticMarkup(
      <SessionItem
        session={session}
        isActive={false}
        isStreaming={true}
        isPinned={true}
        onPinToggle={() => {}}
        onClick={() => {}}
      />,
    );

    expect(html).toContain('session-item-pin--pinned');
    expect(html).toContain('Unpin session');
    expect(html).toContain('session-item-activity');
  });

  it('does not render the pin button when no onPinToggle is provided (e.g. AgentStudio)', () => {
    const html = renderToStaticMarkup(
      <SessionItem
        session={session}
        isActive={false}
        isStreaming={true}
        onClick={() => {}}
      />,
    );

    expect(html).not.toContain('session-item-pin');
    expect(html).toContain('session-item-activity');
  });

  it('renders the pin button when not streaming and onPinToggle is provided', () => {
    const html = renderToStaticMarkup(
      <SessionItem
        session={session}
        isActive={false}
        isStreaming={false}
        onPinToggle={() => {}}
        onClick={() => {}}
      />,
    );

    expect(html).toContain('session-item-pin');
  });

  it('renders neither activity nor pin when not streaming and no pin handler', () => {
    const html = renderToStaticMarkup(
      <SessionItem
        session={session}
        isActive={false}
        isStreaming={false}
        onClick={() => {}}
      />,
    );

    expect(html).not.toContain('session-item-pin');
    expect(html).not.toContain('session-item-activity');
  });
});
