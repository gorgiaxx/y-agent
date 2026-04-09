import type { ReactNode } from 'react';
import type { ToolCallBrief } from '../../../../types';
import type { FormattedResult } from '../toolCallUtils';

export interface ToolRendererProps {
  toolCall: ToolCallBrief;
  status: 'running' | 'success' | 'error';
  result?: string;
  durationMs?: number;
  /** Compact URL metadata JSON from the backend (survives truncation). */
  urlMeta?: string;
  /** Optional structured metadata for tool-specific renderers. */
  metadata?: Record<string, unknown>;
  statusIcon: ReactNode;
  statusClass: string;
  displayArgs: string;
  displayResult: FormattedResult | null;
  displayResultFormatted: FormattedResult | null;
}
