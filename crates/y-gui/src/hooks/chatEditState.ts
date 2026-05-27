import type { PendingEdit } from './useChat';

export function getVisiblePendingEdit(
  pendingEdit: PendingEdit | null,
  activeSessionId: string | null,
): PendingEdit | null {
  if (!pendingEdit) return null;
  if (!pendingEdit.sessionId) return pendingEdit;
  return pendingEdit.sessionId === activeSessionId ? pendingEdit : null;
}
