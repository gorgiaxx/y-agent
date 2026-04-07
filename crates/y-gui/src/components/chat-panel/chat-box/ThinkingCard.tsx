/**
 * ThinkingCard -- lightweight, borderless thinking indicator.
 *
 * Self-contained component (no CollapsibleCard dependency). Shows a pulsing
 * dot animation during streaming and a subtle expandable content block.
 *
 * - During streaming: pulsing dot, "Thinking..." label, collapsed by default
 * - After completion: static dot, "Thought" label, final duration, collapsed
 * - Clicking the header toggles the reasoning content
 */

import { useState, useEffect, useRef } from 'react';
import { ChevronRight } from 'lucide-react';
import './ThinkingCard.css';

interface ThinkingCardProps {
  /** The accumulated reasoning/thinking text. */
  content: string;
  /** Whether this block is still receiving streaming content. */
  isStreaming?: boolean;
  /** Thinking duration in milliseconds (when available from backend). */
  durationMs?: number;
}

/** Format ms as human-readable duration. */
function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  return s < 60 ? `${s.toFixed(1)}s` : `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
}

export function ThinkingCard({ content, isStreaming = false, durationMs }: ThinkingCardProps) {
  // Default collapsed -- user can expand manually.
  const [expanded, setExpanded] = useState(false);
  const [elapsedMs, setElapsedMs] = useState(0);
  const startRef = useRef<number>(Date.now());

  // Live elapsed timer during streaming.
  useEffect(() => {
    if (!isStreaming) return;
    startRef.current = Date.now();
    const timer = setInterval(() => {
      setElapsedMs(Date.now() - startRef.current);
    }, 100);
    return () => clearInterval(timer);
  }, [isStreaming]);

  const displayDuration = isStreaming ? elapsedMs : (durationMs || 0);
  const label = isStreaming ? 'Thinking...' : 'Thought';

  return (
    <div className="thinking-card">
      <div className="thinking-card-header" onClick={() => setExpanded(!expanded)}>
        <span className={`thinking-card-dot${isStreaming ? ' is-streaming' : ''}`} />
        <span className="thinking-card-label">{label}</span>
        {displayDuration > 0 && (
          <span className="thinking-card-duration">{formatDuration(displayDuration)}</span>
        )}
        <span className={`thinking-card-chevron${expanded ? ' expanded' : ''}`}>
          <ChevronRight size={11} />
        </span>
      </div>
      {expanded && content && (
        <div className="thinking-card-body">
          <div className="thinking-card-content">{content}</div>
        </div>
      )}
    </div>
  );
}
