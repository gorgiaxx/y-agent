export interface AnchorRectLike {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

export interface FloatingMenuPosition {
  top: number;
  left: number;
  maxHeight: number;
  direction: 'up' | 'down';
}

export interface SessionPaneLayout {
  showWorkspacePane: boolean;
  showGeneralPane: boolean;
  workspaceMaxHeightRatio: number;
}

const MENU_GAP = 6;
const VIEWPORT_MARGIN = 8;
const WORKSPACE_OPTIONS_SOFT_CAP = 240;
const WORKSPACE_MENU_RESERVED_HEIGHT = 96;

export function calculateFloatingMenuPosition({
  anchorRect,
  menuHeight,
  menuWidth,
  viewportHeight,
  viewportWidth,
}: {
  anchorRect: AnchorRectLike;
  menuHeight: number;
  menuWidth: number;
  viewportHeight: number;
  viewportWidth: number;
}): FloatingMenuPosition {
  const spaceAbove = Math.max(0, anchorRect.top - VIEWPORT_MARGIN - MENU_GAP);
  const spaceBelow = Math.max(0, viewportHeight - anchorRect.bottom - VIEWPORT_MARGIN - MENU_GAP);

  let direction: 'up' | 'down' = 'down';
  if (menuHeight > spaceBelow && spaceAbove > spaceBelow) {
    direction = 'up';
  }

  const availableHeight = Math.max(0, direction === 'up' ? spaceAbove : spaceBelow);
  const renderedHeight = Math.min(menuHeight, availableHeight);
  const unclampedTop = direction === 'up'
    ? anchorRect.top - MENU_GAP - renderedHeight
    : anchorRect.bottom + MENU_GAP;
  const top = Math.max(
    VIEWPORT_MARGIN,
    Math.min(unclampedTop, viewportHeight - VIEWPORT_MARGIN - renderedHeight),
  );

  const preferredLeft = anchorRect.right - menuWidth;
  const maxLeft = Math.max(VIEWPORT_MARGIN, viewportWidth - VIEWPORT_MARGIN - menuWidth);
  const left = Math.max(VIEWPORT_MARGIN, Math.min(preferredLeft, maxLeft));

  return {
    top,
    left,
    maxHeight: availableHeight,
    direction,
  };
}

export function calculateWorkspaceOptionsMaxHeight(menuMaxHeight: number): number {
  return Math.max(
    0,
    Math.min(WORKSPACE_OPTIONS_SOFT_CAP, menuMaxHeight - WORKSPACE_MENU_RESERVED_HEIGHT),
  );
}

export function getSessionPaneLayout({
  workspaceSectionCount,
  ungroupedCount,
}: {
  workspaceSectionCount: number;
  ungroupedCount: number;
}): SessionPaneLayout {
  const showWorkspacePane = workspaceSectionCount > 0;
  return {
    showWorkspacePane,
    showGeneralPane: showWorkspacePane || ungroupedCount > 0,
    workspaceMaxHeightRatio: showWorkspacePane ? 0.5 : 1,
  };
}
