import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { ToolCallCard } from '../components/chat-panel/chat-box/ToolCallCard';

describe('ShellExec tool rendering', () => {
  it('shows a title for background poll actions without a command', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'shell-poll-1',
          name: 'ShellExec',
          arguments: JSON.stringify({
            action: 'poll',
            process_id: 'proc-1',
          }),
        }}
        status="success"
        result={JSON.stringify({
          stdout: 'ready',
          stderr: '',
        })}
      />,
    );

    expect(html).toContain('tool-call-shell-wrapper');
    expect(html).toContain('Poll');
    expect(html).toContain('proc-1');
    expect(html).not.toContain('title=""');
  });
});
