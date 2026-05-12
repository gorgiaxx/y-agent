import { renderToStaticMarkup } from 'react-dom/server';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { forwardRef, type ComponentProps } from 'react';

import { TooltipProvider } from '../components/ui/Tooltip';

vi.stubGlobal('localStorage', {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
});

vi.stubGlobal(
  'EventSource',
  class MockEventSource {
    close() {}
  },
);

vi.mock('../lib', () => ({
  transport: {
    invoke: vi.fn(),
  },
  platform: {
    openFileDialog: vi.fn(),
  },
}));

vi.mock('../components/common/ProviderIconPicker', () => ({
  ProviderIconImg: (props: ComponentProps<'img'>) => <img {...props} alt="" />,
}));

vi.mock('../components/chat-panel/input-area/CommandMenu', () => ({
  CommandMenu: () => null,
}));

vi.mock('../components/chat-panel/input-area/AskUserDialog', () => ({
  AskUserDialog: () => null,
}));

vi.mock('../components/chat-panel/input-area/PermissionDialog', () => ({
  PermissionDialog: () => null,
}));

vi.mock('../components/common/ConfirmDialog', () => ({
  ConfirmDialog: () => null,
}));

vi.mock('../components/chat-panel/input-area/ContentEditableInput', () => ({
  ContentEditableInput: forwardRef(() => <div className="input-content" />),
}));

let InputArea: typeof import('../components/chat-panel/input-area/InputArea').InputArea;

beforeAll(async () => {
  ({ InputArea } = await import('../components/chat-panel/input-area/InputArea'));
});

describe('InputArea toolbar tooltips', () => {
  it('does not rely on CSS-only data-tooltip attributes for toolbar actions', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <InputArea
          onSend={() => {}}
          disabled={false}
          sendOnEnter
          provider={{
            providers: [],
            selectedProviderId: 'auto',
            onSelectProvider: () => {},
          }}
          mcp={{}}
          dialogs={{}}
          edit={{}}
          features={{}}
        />
      </TooltipProvider>,
    );

    expect(html).not.toContain('data-tooltip=');
    expect(html).toContain('aria-label="Select model"');
    expect(html).toContain('aria-label="Mode: fast"');
    expect(html).toContain('aria-label="Operation mode: Default permissions"');
    expect(html).toContain('aria-label="Attach images"');
    expect(html).toContain('aria-label="Expand input"');
  });
});
