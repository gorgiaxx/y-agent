import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { ToolCallCard } from '../components/chat-panel/chat-box/ToolCallCard';

describe('File tool rendering', () => {
  it('renders file write calls with the file tag layout', () => {
    const html = renderToStaticMarkup(
      <ToolCallCard
        toolCall={{
          id: 'file-write-1',
          name: 'FileWrite',
          arguments: JSON.stringify({
            path: '/tmp/example.ts',
            content: 'export const value = 1;\n',
          }),
        }}
        status="success"
        result={JSON.stringify({
          ok: true,
          path: '/tmp/example.ts',
        })}
      />,
    );

    expect(html).toContain('tool-call-file-tag');
    expect(html).toContain('Create');
    expect(html).toContain('example.ts');
    expect(html).not.toContain('tool-call-card');
  });
});
