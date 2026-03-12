import './StatusBar.css';

interface StatusBarProps {
  providerCount: number;
  sessionCount: number | null;
  version: string;
  activeModel?: string;
  lastCost?: number;
  lastTokens?: { input: number; output: number };
}

export function StatusBar({
  providerCount,
  version,
  activeModel,
  lastCost,
  lastTokens,
}: StatusBarProps) {
  return (
    <div className="status-bar">
      <div className="status-left">
        {activeModel && (
          <span className="status-item status-model">
            <span className="status-dot success" />
            {activeModel}
          </span>
        )}
        {lastTokens && (
          <span className="status-item">
            {(lastTokens.input + lastTokens.output).toLocaleString()} tokens
          </span>
        )}
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
