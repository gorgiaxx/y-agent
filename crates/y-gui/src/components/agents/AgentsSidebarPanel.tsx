import { Plus, RefreshCw, Search } from 'lucide-react';
import type { AgentInfo } from '../../hooks/useAgents';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { ScrollArea } from '../ui/ScrollArea';
import {
  AGENT_TIER_ORDER,
  AgentGlyph,
  formatAgentModeLabel,
  formatAgentTierHint,
  formatAgentTierLabel,
  getAgentModeBadgeVariant,
} from './agentDisplay';

interface AgentsSidebarPanelProps {
  agents: AgentInfo[];
  activeAgentId: string | null;
  query: string;
  totalCount: number;
  reloading?: boolean;
  onQueryChange: (value: string) => void;
  onSelectAgent: (id: string) => void;
  onReload?: () => void;
  onNewAgent?: () => void;
}

export function AgentsSidebarPanel({
  agents,
  activeAgentId,
  query,
  totalCount,
  reloading,
  onQueryChange,
  onSelectAgent,
  onReload,
  onNewAgent,
}: AgentsSidebarPanelProps) {
  const groupedAgents = AGENT_TIER_ORDER
    .map((tier) => ({
      tier,
      agents: agents.filter((agent) => agent.trust_tier === tier),
    }))
    .filter((group) => group.agents.length > 0);

  return (
    <aside className="agents-sidebar-panel">
      <div className="agents-sidebar-panel-header">
        <div className="agents-sidebar-header-row">
          <div className="agents-sidebar-count-row">
            <span className="agents-sidebar-count">{agents.length}</span>
            <span className="agents-sidebar-count-caption">
              {query.trim() ? `matching ${totalCount} total presets` : 'available presets'}
            </span>
          </div>

          <div className="agents-sidebar-actions">
            {onReload && (
              <Button
                variant="icon"
                size="sm"
                onClick={() => onReload()}
                disabled={reloading}
                title="Reload"
              >
                <RefreshCw size={14} className={reloading ? 'agents-spin' : ''} />
              </Button>
            )}
            {onNewAgent && (
              <Button
                variant="icon"
                size="sm"
                onClick={() => onNewAgent()}
                title="New Agent"
              >
                <Plus size={14} />
              </Button>
            )}
          </div>
        </div>

        <label className="agents-sidebar-search">
          <Search size={14} className="agents-sidebar-search-icon" />
          <Input
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="Search agents"
            className="agents-sidebar-search-input"
          />
        </label>
      </div>

      <ScrollArea className="flex-1 min-h-0">
        <div className="agents-sidebar-panel-body">
          {groupedAgents.length === 0 ? (
            <div className="agents-sidebar-empty">
              No agents match the current filter.
            </div>
          ) : (
            groupedAgents.map((group) => (
              <section key={group.tier} className="agents-sidebar-group">
                <div className="agents-sidebar-group-header">
                  <div>
                    <div className="agents-sidebar-group-title">
                      {formatAgentTierLabel(group.tier)}
                    </div>
                    <div className="agents-sidebar-group-hint">
                      {formatAgentTierHint(group.tier)}
                    </div>
                  </div>
                  <Badge variant="outline">{group.agents.length}</Badge>
                </div>

                <div className="agents-sidebar-group-items">
                  {group.agents.map((agent) => {
                    const isActive = activeAgentId === agent.id;

                    return (
                      <button
                        key={agent.id}
                        type="button"
                        className={[
                          'agents-sidebar-item',
                          isActive ? 'agents-sidebar-item--active' : '',
                        ].filter(Boolean).join(' ')}
                        onClick={() => onSelectAgent(agent.id)}
                      >
                        <div className="agents-sidebar-item-glyph">
                          <AgentGlyph id={agent.id} name={agent.name} size={15} />
                        </div>

                        <div className="agents-sidebar-item-body">
                          <div className="agents-sidebar-item-top">
                            <span className="agents-sidebar-item-name">{agent.name}</span>
                            {agent.is_overridden && (
                              <Badge variant="accent">override</Badge>
                            )}
                          </div>

                          <p className="agents-sidebar-item-desc">{agent.description}</p>

                          <div className="agents-sidebar-item-meta">
                            <Badge variant={getAgentModeBadgeVariant(agent.mode)}>
                              {formatAgentModeLabel(agent.mode)}
                            </Badge>
                            {!agent.user_callable && (
                              <Badge variant="outline">hidden</Badge>
                            )}
                          </div>
                        </div>
                      </button>
                    );
                  })}
                </div>
              </section>
            ))
          )}
        </div>
      </ScrollArea>
    </aside>
  );
}
