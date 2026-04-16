// ObservabilityPanel -- live system state visualization.

import { useState, useRef, useEffect } from 'react';
import { Eye, X, Maximize2, Minimize2, Server, Bot, ChevronDown, ChevronRight, Filter } from 'lucide-react';
import { ProviderIconImg } from '../common/ProviderIconPicker';
import { Button } from '../ui';

import type { SystemSnapshot, ProviderSnapshot as ProviderSnap, AgentInstanceSnapshot } from '../../types';
import type { TimeRange } from '../../hooks/useObservability';
import './ObservabilityPanel.css';

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function ProviderCard({ provider, iconId }: { provider: ProviderSnap; iconId?: string | null }) {
  const pct = provider.max_concurrency > 0
    ? (provider.active_requests / provider.max_concurrency) * 100
    : 0;
  const fillClass = pct >= 100 ? 'full' : pct >= 75 ? 'high' : '';

  return (
    <div className="obs-provider-card">
      <div className="obs-provider-identity">
        <div className="obs-provider-icon">
          {iconId ? (
            <ProviderIconImg iconId={iconId} size={12} />
          ) : (
            <Server size={12} />
          )}
        </div>
        <span className="obs-provider-name">{provider.id}</span>
        <span className="obs-provider-model">{provider.model}</span>
        <span className={`obs-badge ${provider.is_frozen ? 'obs-badge-frozen' : 'obs-badge-healthy'}`}>
          {provider.is_frozen ? 'FROZEN' : 'OK'}
        </span>
      </div>

      {provider.tags.length > 0 && (
        <div className="obs-tags">
          {provider.tags.map((tag) => (
            <span key={tag} className="obs-tag">{tag}</span>
          ))}
        </div>
      )}

      <div className="obs-concurrency">
        <div className="obs-concurrency-label">
          <span className="obs-concurrency-text">CONCURRENCY</span>
          <span className="obs-concurrency-text">{provider.active_requests} / {provider.max_concurrency}</span>
        </div>
        <div className="obs-concurrency-bar">
          <div
            className={`obs-concurrency-fill ${fillClass}`}
            style={{ width: `${Math.min(pct, 100)}%` }}
          />
        </div>
      </div>

      <div className="obs-metrics">
        <div className="obs-metric">
          <span className="obs-metric-label">Requests</span>
          <span className="obs-metric-value">{provider.total_requests.toLocaleString()}</span>
        </div>
        <div className="obs-metric">
          <span className="obs-metric-label">Errors</span>
          <span className="obs-metric-value">{provider.total_errors.toLocaleString()}</span>
        </div>
        <div className="obs-metric">
          <span className="obs-metric-label">Err Rate</span>
          <span className="obs-metric-value">{(provider.error_rate * 100).toFixed(1)}%</span>
        </div>
        <div className="obs-metric">
          <span className="obs-metric-label">In Tokens</span>
          <span className="obs-metric-value">{provider.total_input_tokens.toLocaleString()}</span>
        </div>
        <div className="obs-metric">
          <span className="obs-metric-label">Out Tokens</span>
          <span className="obs-metric-value">{provider.total_output_tokens.toLocaleString()}</span>
        </div>
        <div className="obs-metric">
          <span className="obs-metric-label">Cost</span>
          <span className="obs-metric-value">${provider.estimated_cost_usd.toFixed(4)}</span>
        </div>
      </div>
    </div>
  );
}

function stateClass(state: string): string {
  const s = state.toLowerCase();
  if (s === 'running') return 'state-running';
  if (s === 'creating') return 'state-creating';
  if (s === 'configuring') return 'state-configuring';
  if (s === 'completed') return 'state-completed';
  if (s === 'failed') return 'state-failed';
  if (s === 'interrupted') return 'state-interrupted';
  return '';
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${(ms / 60_000).toFixed(1)}m`;
}

function AgentCard({ agent }: { agent: AgentInstanceSnapshot }) {
  const sc = stateClass(agent.state);

  return (
    <div className="obs-agent-card">
      <div className="obs-agent-header">
        <div className={`obs-agent-icon ${sc}`}>
          <Bot size={12} />
        </div>
        <span className="obs-agent-name">{agent.agent_name}</span>
        <span className={`obs-agent-state ${sc}`}>{agent.state}</span>
      </div>
      <div className="obs-agent-details">
        <div className="obs-metric">
          <span className="obs-metric-label">Elapsed</span>
          <span className="obs-metric-value">{formatDuration(agent.elapsed_ms)}</span>
        </div>
        <div className="obs-metric">
          <span className="obs-metric-label">Iterations</span>
          <span className="obs-metric-value">{agent.iterations}</span>
        </div>
        <div className="obs-metric">
          <span className="obs-metric-label">Tokens</span>
          <span className="obs-metric-value">{agent.tokens_used.toLocaleString()}</span>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Panel
// ---------------------------------------------------------------------------

interface ObservabilityPanelProps {
  snapshot: SystemSnapshot | null;
  loading: boolean;
  error: string | null;
  expanded: boolean;
  onToggleExpand: () => void;
  onClose: () => void;
  timeRange: TimeRange;
  onTimeRangeChange: (range: TimeRange) => void;
  /** Map from provider ID to icon identifier. */
  providerIcons?: Record<string, string>;
}

const TIME_RANGE_OPTIONS: { value: TimeRange; label: string }[] = [
  { value: '15m', label: '15 min' },
  { value: '30m', label: '30 min' },
  { value: '1h', label: '1 hour' },
  { value: '6h', label: '6 hours' },
  { value: '24h', label: '24 hours' },
  { value: 'all', label: 'All time' },
];

export function ObservabilityPanel({ snapshot, loading, error, expanded, onToggleExpand, onClose, timeRange, onTimeRangeChange, providerIcons }: ObservabilityPanelProps) {
  const [providersOpen, setProvidersOpen] = useState(true);
  const [agentsOpen, setAgentsOpen] = useState(true);
  const [filterOpen, setFilterOpen] = useState(false);
  const filterRef = useRef<HTMLDivElement>(null);

  // Close filter popover on click outside.
  useEffect(() => {
    if (!filterOpen) return;
    const handleClick = (e: MouseEvent) => {
      if (filterRef.current && !filterRef.current.contains(e.target as Node)) {
        setFilterOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [filterOpen]);

  const providerCount = snapshot?.providers.length ?? 0;
  const activeAgents = snapshot?.agents.active_instances ?? 0;
  const availableSlots = snapshot?.agents.available_slots ?? 0;

  const panelContent = (
    <div className={`obs-panel ${expanded ? 'obs-expanded' : ''}`}>
      {/* Header */}
      <div className="obs-header">
        <div className="obs-header-left">
          <Eye size={16} className="obs-header-icon" />
          <h3 className="obs-title">Observability</h3>
          {loading && !snapshot && <span className="obs-summary-label">loading...</span>}
        </div>
        <div className="obs-header-actions">
          <div className="obs-filter-wrapper" ref={filterRef}>
            <Button
              variant="icon"
              size="sm"
              className={timeRange !== 'all' ? 'obs-btn-active' : ''}
              onClick={() => setFilterOpen(!filterOpen)}
              title="Filter by time range"
            >
              <Filter size={14} />
            </Button>
            {filterOpen && (
              <div className="obs-filter-popover">
                <div className="obs-filter-title">Time range</div>
                {TIME_RANGE_OPTIONS.map((opt) => (
                  <button
                    key={opt.value}
                    className={`obs-filter-option${timeRange === opt.value ? ' active' : ''}`}
                    onClick={() => {
                      onTimeRangeChange(opt.value);
                      setFilterOpen(false);
                    }}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
            )}
          </div>
          <Button variant="icon" size="sm" onClick={onToggleExpand} title={expanded ? 'Collapse' : 'Expand'}>
            {expanded ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
          </Button>
          <Button variant="icon" size="sm" onClick={onClose} title="Close">
            <X size={14} />
          </Button>
        </div>
      </div>

      {/* Summary bar */}
      {snapshot && (
        <div className="obs-summary">
          <div className="obs-summary-item">
            <span className="obs-summary-value">{providerCount}</span>
            <span className="obs-summary-label">providers</span>
          </div>
          <div className="obs-summary-item">
            <span className="obs-summary-value">{activeAgents}</span>
            <span className="obs-summary-label">agents</span>
          </div>
          <div className="obs-summary-item">
            <span className="obs-summary-value">{availableSlots}</span>
            <span className="obs-summary-label">slots</span>
          </div>
        </div>
      )}

      {/* Content */}
      <div className="obs-content">
        {!snapshot ? (
          <div className="obs-empty">
            <Eye size={24} className="obs-empty-icon" />
            {error ? (
              <>
                <p className="obs-empty-text">Failed to load system state</p>
                <p className="obs-empty-hint">{error}</p>
              </>
            ) : (
              <>
                <p className="obs-empty-text">Loading system state...</p>
                <p className="obs-empty-hint">Fetching provider and agent information.</p>
              </>
            )}
          </div>
        ) : (
          <>
            {/* Providers Section */}
            <div className="obs-section">
              <div className="obs-section-header" onClick={() => setProvidersOpen(!providersOpen)}>
                <span className="obs-section-chevron">
                  {providersOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                </span>
                <span className="obs-section-title">Provider Pool</span>
                <span className="obs-section-count">{providerCount}</span>
              </div>
              {providersOpen && (
                snapshot.providers.length === 0 ? (
                  <div className="obs-no-items">No providers configured</div>
                ) : (
                  snapshot.providers.map((p) => (
                    <ProviderCard key={p.id} provider={p} iconId={providerIcons?.[p.id]} />
                  ))
                )
              )}
            </div>

            {/* Agents Section */}
            <div className="obs-section">
              <div className="obs-section-header" onClick={() => setAgentsOpen(!agentsOpen)}>
                <span className="obs-section-chevron">
                  {agentsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                </span>
                <span className="obs-section-title">Agent Pool</span>
                <span className="obs-section-count">{snapshot.agents.total_instances}</span>
              </div>
              {agentsOpen && (
                snapshot.agents.instances.length === 0 ? (
                  <div className="obs-no-items">No agent instances</div>
                ) : (
                  snapshot.agents.instances.map((a) => (
                    <AgentCard key={a.instance_id} agent={a} />
                  ))
                )
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );

  if (expanded) {
    return (
      <div className="obs-backdrop" onClick={onClose}>
        <div onClick={(e) => e.stopPropagation()}>
          {panelContent}
        </div>
      </div>
    );
  }

  return panelContent;
}
