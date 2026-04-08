import { useState } from 'react';
import { Wrench } from 'lucide-react';
import { formatDuration } from '../../../../utils/formatDuration';
import { CollapsibleCard } from '../CollapsibleCard';
import { DetailSections } from './shared';
import type { ToolRendererProps } from './types';

const ACCENT_COLOR = 'var(--accent)';

export function DefaultRenderer({
  toolCall, status, durationMs,
  statusIcon, statusClass,
  displayArgs, displayResult, displayResultFormatted,
}: ToolRendererProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  const activeResult = showRaw ? displayResult : (displayResultFormatted ?? displayResult);
  const hasExpandable = displayArgs || displayResult;

  const statusLabel = { running: 'Running...', success: 'Done', error: 'Failed' }[status];

  const headerRight = (
    <>
      <span className={`tool-call-status-icon ${statusClass}`}>{statusIcon}</span>
      <span className={`tool-call-status ${statusClass}`}>{statusLabel}</span>
      {durationMs !== undefined && (
        <span className="tool-call-duration">{formatDuration(durationMs)}</span>
      )}
    </>
  );

  return (
    <CollapsibleCard
      icon={<Wrench size={12} />}
      label={<span className="tool-call-name">{toolCall.name}</span>}
      accentColor={ACCENT_COLOR}
      expanded={expanded}
      onToggle={() => hasExpandable && setExpanded(!expanded)}
      headerRight={headerRight}
      className="tool-call-card"
    >
      <DetailSections
        displayArgs={displayArgs}
        displayResult={activeResult}
        showRaw={showRaw}
        onToggleRaw={() => setShowRaw(!showRaw)}
      />
    </CollapsibleCard>
  );
}
