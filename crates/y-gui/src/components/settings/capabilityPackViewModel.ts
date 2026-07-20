import type { SessionInfo } from '../../types';
import type { SearchableOption } from '../common/searchableSelectUtils';

export function buildApprovalSessionOptions(sessions: SessionInfo[]): SearchableOption[] {
  return sessions.map((session) => ({
    value: session.id,
    label: session.manual_title?.trim() || session.title?.trim() || 'Untitled session',
    description: session.id,
    keywords: [session.id, session.agent_id ?? '', session.created_at],
  }));
}
