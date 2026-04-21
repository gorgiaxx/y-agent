export const MIN_DIAGNOSTICS_PANEL_WIDTH = 320;
export const DEFAULT_DIAGNOSTICS_PANEL_WIDTH = 380;
export const MAX_DIAGNOSTICS_PANEL_WIDTH = 720;

const MIN_REMAINING_APP_WIDTH = 420;

export interface DiagnosticsPanelPointer {
  clientX: number;
  viewportWidth: number;
}

export function constrainDiagnosticsPanelWidth(width: number, viewportWidth: number): number {
  const remainingWidthMax = Number.isFinite(viewportWidth)
    ? viewportWidth - MIN_REMAINING_APP_WIDTH
    : MAX_DIAGNOSTICS_PANEL_WIDTH;
  const maxWidth = Math.max(
    MIN_DIAGNOSTICS_PANEL_WIDTH,
    Math.min(MAX_DIAGNOSTICS_PANEL_WIDTH, Math.floor(remainingWidthMax)),
  );
  const safeWidth = Number.isFinite(width) ? Math.round(width) : DEFAULT_DIAGNOSTICS_PANEL_WIDTH;

  return Math.min(Math.max(safeWidth, MIN_DIAGNOSTICS_PANEL_WIDTH), maxWidth);
}

export function diagnosticsPanelWidthFromPointer(pointer: DiagnosticsPanelPointer): number {
  return constrainDiagnosticsPanelWidth(pointer.viewportWidth - pointer.clientX, pointer.viewportWidth);
}
