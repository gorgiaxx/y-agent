import { Bot, SquareTerminal } from 'lucide-react';
import { ProviderIconImg } from '../common/ProviderIconPicker';
import { useConnectionStatus } from '../../hooks/useConnectionStatus';
import { platform } from '../../lib';
import './StatusBar.css';

interface StatusBarProps {
  version: string;
  activeModel?: string;
  activeProviderIcon?: string | null;
  lastCost?: number;
  lastTokens?: { input: number; output: number };
  contextWindow?: number;
  /** Actual context occupancy from last LLM call's prompt tokens. */
  contextTokensUsed?: number;
  backgroundTasks?: {
    total: number;
    running: number;
    failed: number;
    onClick: () => void;
  };
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function taskCountLabel(count: number): string {
  return `${count} task${count === 1 ? '' : 's'}`;
}

function taskStatusLabel(total: number, running: number, failed: number): string {
  if (running > 0) return `${running} running, ${taskCountLabel(total)}`;
  if (failed > 0) return `${failed} failed, ${taskCountLabel(total)}`;
  return taskCountLabel(total);
}

export function StatusBar({
  version,
  activeModel,
  activeProviderIcon,
  lastCost,
  lastTokens,
  contextWindow,
  contextTokensUsed,
  backgroundTasks,
}: StatusBarProps) {
  const connStatus = useConnectionStatus();
  const showConnectionStatus = platform.capabilities.sseEvents;
  // Context occupancy: prefer the explicit context_tokens_used (last iteration's
  // prompt size), fall back to cumulative input tokens if not available.
  const occupancy = contextTokensUsed ?? (lastTokens ? lastTokens.input : 0);
  const pct =
    contextWindow && contextWindow > 0 ? Math.min((occupancy / contextWindow) * 100, 100) : null;
  const taskLabel = backgroundTasks
    ? taskStatusLabel(backgroundTasks.total, backgroundTasks.running, backgroundTasks.failed)
    : null;

  return (
    <div className="status-bar">
      <div className="status-left">
        {activeModel && (
          <span className="status-item status-model">
            {activeProviderIcon ? (
              <ProviderIconImg iconId={activeProviderIcon} size={14} className="status-model-icon" />
            ) : (
              <Bot size={14} className="status-model-icon status-model-icon--default" />
            )}
            {activeModel}
          </span>
        )}
        {contextWindow && contextWindow > 0 && occupancy > 0 ? (
          <span className="status-item status-tokens">
            <span className="status-token-ratio">
              {formatTokens(occupancy)}/{formatTokens(contextWindow)}
            </span>
            <span className="status-token-pct">({pct!.toFixed(1)}%)</span>
            <span className="status-token-bar" title={`${pct!.toFixed(1)}% context used`}>
              <span
                className={`status-token-fill${pct! > 80 ? ' warn' : ''}`}
                style={{ width: `${pct}%` }}
              />
            </span>
          </span>
        ) : lastTokens ? (
          <span className="status-item">
            {(lastTokens.input + lastTokens.output).toLocaleString()} tokens
          </span>
        ) : null}
        {lastCost !== undefined && lastCost > 0 && (
          <span className="status-item">${lastCost.toFixed(4)}</span>
        )}
      </div>
      <div className="status-right">
        {backgroundTasks && taskLabel && (
          <button
            type="button"
            className={`status-item status-action status-background-tasks ${
              backgroundTasks.running > 0
                ? 'status-background-tasks--running'
                : backgroundTasks.failed > 0
                  ? 'status-background-tasks--failed'
                  : ''
            }`}
            onClick={backgroundTasks.onClick}
            title="Open background tasks"
            aria-label={`Open background tasks (${taskLabel})`}
          >
            <SquareTerminal size={13} />
            <span>{taskLabel}</span>
          </button>
        )}
        {showConnectionStatus && (
          <span className={`status-item status-connection status-connection--${connStatus}`}>
            <span className="status-connection-dot" />
            {connStatus === 'connected' ? 'Online' : connStatus === 'connecting' ? 'Connecting...' : 'Offline'}
          </span>
        )}
        <span className="status-item status-version">v{version}</span>
      </div>
    </div>
  );
}
