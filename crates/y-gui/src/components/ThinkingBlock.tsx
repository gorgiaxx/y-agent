import { useState, useEffect, useRef } from 'react';
import { Brain, ChevronRight, Loader } from 'lucide-react';
import './ThinkingBlock.css';

interface ThinkingBlockProps {
  /** The accumulated reasoning/thinking text. */
  content: string;
  /** Whether this block is still receiving streaming content. */
  isStreaming?: boolean;
  /** Thinking duration in milliseconds (when available). */
  durationMs?: number;
}

/** Format ms as human-readable duration. */
function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  return s < 60 ? `${s.toFixed(1)}s` : `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
}

/**
 * Collapsible "Thinking" block rendered inside an assistant message bubble.
 *
 * - During streaming: expanded, spinning icon, live-updating elapsed time
 * - After completion: auto-collapses, shows final duration
 */
export function ThinkingBlock({ content, isStreaming = false, durationMs }: ThinkingBlockProps) {
  const [expanded, setExpanded] = useState(isStreaming || !durationMs);
  const [elapsedMs, setElapsedMs] = useState(0);
  const startRef = useRef<number>(Date.now());

  // Auto-collapse when streaming finishes.
  useEffect(() => {
    if (!isStreaming && expanded) {
      setExpanded(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isStreaming]);

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

  return (
    <div className="thinking-block">
      <div className="thinking-header" onClick={() => setExpanded(!expanded)}>
        <span className="thinking-icon">
          {isStreaming ? <Loader size={13} className="thinking-spinner" /> : <Brain size={13} />}
        </span>
        <span className="thinking-label">
          {isStreaming ? 'Thinking…' : 'Thought'}
        </span>
        {displayDuration > 0 && (
          <span className="thinking-duration">{formatDuration(displayDuration)}</span>
        )}
        <span className={`thinking-expand ${expanded ? 'expanded' : ''}`}>
          <ChevronRight size={12} />
        </span>
      </div>
      {expanded && (
        <div className="thinking-body">
          <div className="thinking-content">{content}</div>
        </div>
      )}
    </div>
  );
}
