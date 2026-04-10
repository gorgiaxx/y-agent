import { describe, expect, it } from 'vitest';

import {
  calculateFloatingMenuPosition,
  calculateWorkspaceOptionsMaxHeight,
  getSessionPaneLayout,
} from '../components/chat-panel/chatSidebarLayout';

describe('calculateFloatingMenuPosition', () => {
  it('flips upward when there is not enough space below the anchor', () => {
    const position = calculateFloatingMenuPosition({
      anchorRect: { top: 560, right: 236, bottom: 584, left: 212 },
      menuHeight: 220,
      menuWidth: 180,
      viewportHeight: 600,
      viewportWidth: 320,
    });

    expect(position.direction).toBe('up');
    expect(position.top).toBeLessThan(560);
    expect(position.maxHeight).toBe(546);
  });

  it('caps menu height to the available viewport space', () => {
    const position = calculateFloatingMenuPosition({
      anchorRect: { top: 40, right: 284, bottom: 64, left: 260 },
      menuHeight: 480,
      menuWidth: 220,
      viewportHeight: 260,
      viewportWidth: 320,
    });

    expect(position.direction).toBe('down');
    expect(position.maxHeight).toBe(182);
  });
});

describe('calculateWorkspaceOptionsMaxHeight', () => {
  it('keeps the workspace list scrollable instead of letting the menu grow without limit', () => {
    expect(calculateWorkspaceOptionsMaxHeight(520)).toBe(240);
    expect(calculateWorkspaceOptionsMaxHeight(180)).toBe(84);
  });
});

describe('getSessionPaneLayout', () => {
  it('caps the workspace pane at half height and still reserves room for general sessions', () => {
    expect(getSessionPaneLayout({ workspaceSectionCount: 2, ungroupedCount: 0 })).toEqual({
      showWorkspacePane: true,
      showGeneralPane: true,
      workspaceMaxHeightRatio: 0.5,
    });
  });
});
