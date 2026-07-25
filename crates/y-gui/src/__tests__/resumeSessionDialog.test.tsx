import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import {
  ResumeSessionDialogContent,
} from '../components/chat-panel/ResumeSessionDialog';
import { nextResumeSelection } from '../components/chat-panel/resumeSessionDialogState';
import type { SessionInfo } from '../types';

const sessions: SessionInfo[] = [
  {
    id: 'session-current-12345678',
    title: 'Generated title',
    manual_title: 'Workspace resume redesign',
    workspace_path: '/Users/rin/Projects/y-agent',
    created_at: '2026-07-25T01:00:00Z',
    updated_at: '2026-07-25T02:50:00Z',
    message_count: 12,
  },
  {
    id: 'session-other-87654321',
    title: 'Storage migration',
    workspace_path: '/Users/rin/Projects/y-agent',
    created_at: '2026-07-24T01:00:00Z',
    updated_at: '2026-07-24T01:00:00Z',
    message_count: 4,
  },
];

describe('ResumeSessionDialog', () => {
  it('renders the shared picker hierarchy and useful session metadata', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-25T03:00:00Z'));

    const html = renderToStaticMarkup(
      <ResumeSessionDialogContent
        workspacePath="/Users/rin/Projects/y-agent"
        currentSessionId="session-current-12345678"
        sessions={sessions}
        loading={false}
        error={null}
        onSelect={() => {}}
        onClose={() => {}}
      />,
    );

    expect(html).toContain('resume-dialog-header');
    expect(html).toContain('aria-label="Close resume session picker"');
    expect(html).toContain('y-agent');
    expect(html).toContain('title="/Users/rin/Projects/y-agent"');
    expect(html).toContain('Workspace resume redesign');
    expect(html).toContain('resume-session-item--current');
    expect(html).toContain('Current');
    expect(html).toContain('12 messages');
    expect(html).toContain('10m');
    expect(html).toContain('Navigate');
    expect(html).toContain('Resume');

    vi.useRealTimers();
  });

  it('wraps keyboard selection through the filtered list', () => {
    expect(nextResumeSelection(0, 2, 'ArrowDown')).toBe(1);
    expect(nextResumeSelection(1, 2, 'ArrowDown')).toBe(0);
    expect(nextResumeSelection(0, 2, 'ArrowUp')).toBe(1);
    expect(nextResumeSelection(0, 0, 'ArrowDown')).toBe(0);
  });
});
