import './StatusBar.css';

interface StatusBarProps {
  providerCount: number;
  sessionCount: number | null;
  version: string;
  activeModel?: string;
  lastCost?: number;
  lastTokens?: { input: number; output: number };
  contextWindow?: number;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function StatusBar({
  providerCount,
  version,
  activeModel,
  lastCost,
  lastTokens,
  contextWindow,
}: StatusBarProps) {
  const usedTokens = lastTokens ? lastTokens.input + lastTokens.output : 0;
  const pct =
    contextWindow && contextWindow > 0 ? Math.min((usedTokens / contextWindow) * 100, 100) : null;

  return (
    <div className="status-bar">
      <div className="status-left">
        {activeModel && (
          <span className="status-item status-model">
            <span className="status-dot success" />
            {activeModel}
          </span>
        )}
        {lastTokens && contextWindow && contextWindow > 0 ? (
          <span className="status-item status-tokens">
            <span className="status-token-ratio">
              {formatTokens(usedTokens)}/{formatTokens(contextWindow)}
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
        <span className="status-item">
          {providerCount} provider{providerCount !== 1 ? 's' : ''}
        </span>
        <span className="status-item status-version">v{version}</span>
      </div>
    </div>
  );
}
