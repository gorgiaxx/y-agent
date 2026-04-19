import { Bot } from 'lucide-react';
import { ProviderIconImg } from '../common/ProviderIconPicker';
import { useConnectionStatus } from '../../hooks/useConnectionStatus';
import { platform } from '../../lib';
import './StatusBar.css';

interface StatusBarProps {
  providerCount: number;
  sessionCount: number | null;
  version: string;
  activeModel?: string;
  activeProviderIcon?: string | null;
  lastCost?: number;
  lastTokens?: { input: number; output: number };
  contextWindow?: number;
  /** Actual context occupancy from last LLM call's prompt tokens. */
  contextTokensUsed?: number;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function StatusBar({
  version,
  activeModel,
  activeProviderIcon,
  lastCost,
  lastTokens,
  contextWindow,
  contextTokensUsed,
}: StatusBarProps) {
  const connStatus = useConnectionStatus();
  const isWeb = !platform.isTauri();
  // Context occupancy: prefer the explicit context_tokens_used (last iteration's
  // prompt size), fall back to cumulative input tokens if not available.
  const occupancy = contextTokensUsed ?? (lastTokens ? lastTokens.input : 0);
  const pct =
    contextWindow && contextWindow > 0 ? Math.min((occupancy / contextWindow) * 100, 100) : null;

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
        {isWeb && (
          <span className={`status-item status-connection status-connection--${connStatus}`}>
            <span className="status-connection-dot" />
            {connStatus === 'connected' ? 'Online' : connStatus === 'connecting' ? 'Connecting...' : 'Offline'}
          </span>
        )}
        {/* <span className="status-item">
          {providerCount} provider{providerCount !== 1 ? 's' : ''}
        </span> */}
        <span className="status-item status-version">v{version}</span>
      </div>
    </div>
  );
}
