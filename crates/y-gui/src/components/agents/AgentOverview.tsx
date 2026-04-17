import { Plus, RefreshCw, Search, SquarePen } from 'lucide-react';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { ScrollArea } from '../ui/ScrollArea';
import type { AgentInfo } from '../../hooks/useAgents';
import { AgentGlyph } from './AgentGlyph';
import {
  formatAgentModeLabel,
  formatAgentTierLabel,
  getAgentModeBadgeVariant,
} from './agentDisplay';

interface AgentOverviewProps {
  filteredAgents: AgentInfo[];
  totalCount: number;
  agentQuery: string;
  reloading?: boolean;
  onQueryChange: (value: string) => void;
  onSelectAgent: (id: string) => void;
  onOpenEdit: (id: string) => void;
  onReload?: () => void;
  onNewAgent?: () => void;
}

export function AgentOverview({
  filteredAgents,
  totalCount,
  agentQuery,
  reloading,
  onQueryChange,
  onSelectAgent,
  onOpenEdit,
  onReload,
  onNewAgent,
}: AgentOverviewProps) {
  return (
    <ScrollArea className="flex-1 min-h-0">
      <div className="agents-overview">
        <div className="agents-overview-toolbar">
          <label className="agents-overview-search">
            <Search size={14} className="agents-overview-search-icon" />
            <Input
              value={agentQuery}
              onChange={(event) => onQueryChange(event.target.value)}
              placeholder="Search agents"
              className="agents-overview-search-input"
            />
          </label>

          <div className="agents-overview-actions">
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

        <section className="agents-gallery-section">
          <div className="agents-gallery-header">
            <div>
              <div className="agents-section-kicker">
                {agentQuery.trim() ? 'Filtered Results' : 'Preset Catalog'}
              </div>
              <h3 className="agents-gallery-title">
                {agentQuery.trim()
                  ? `${filteredAgents.length} of ${totalCount} presets`
                  : 'Choose a preset to open its workspace'}
              </h3>
            </div>
          </div>

          {filteredAgents.length === 0 ? (
            <div className="agents-gallery-empty">
              No agents match the current search. Try a different name, mode, or provider.
            </div>
          ) : (
            <div className="agents-gallery-grid">
              {filteredAgents.map((agent) => (
                <AgentPresetCard
                  key={agent.id}
                  agent={agent}
                  onOpen={() => onSelectAgent(agent.id)}
                  onEdit={() => onOpenEdit(agent.id)}
                />
              ))}
            </div>
          )}
        </section>
      </div>
    </ScrollArea>
  );
}

interface AgentPresetCardProps {
  agent: AgentInfo;
  onOpen: () => void;
  onEdit: () => void;
}

function AgentPresetCard({ agent, onOpen, onEdit }: AgentPresetCardProps) {
  return (
    <article className="agents-preset-card" onClick={onOpen}>
      <div className="agents-preset-card-top">
        <div className="agents-preset-card-glyph">
          <AgentGlyph id={agent.id} name={agent.name} size={16} />
        </div>
        <div className="agents-preset-card-badges">
          <Badge variant={getAgentModeBadgeVariant(agent.mode)}>
            {formatAgentModeLabel(agent.mode)}
          </Badge>
          {agent.is_overridden && (
            <Badge variant="accent">override</Badge>
          )}
        </div>
        <Button
          variant="icon"
          size="sm"
          onClick={(e) => {
            e.stopPropagation();
            onEdit();
          }}
          title="Edit preset"
        >
          <SquarePen size={12} />
        </Button>
      </div>

      <div className="agents-preset-card-body">
        <div className="agents-preset-card-title-row">
          <h4 className="agents-preset-card-title">{agent.name}</h4>
          <span className="agents-preset-card-tier">
            {formatAgentTierLabel(agent.trust_tier)}
          </span>
        </div>
        <p className="agents-preset-card-description">{agent.description}</p>
      </div>

      <div className="agents-preset-card-footer">
        <div className="agents-preset-card-badges">
          <Badge variant="outline">{agent.provider_id || 'auto'}</Badge>
          {agent.features.toolcall && <Badge variant="success">tools</Badge>}
          {agent.features.skills && <Badge variant="accent">skills</Badge>}
          {agent.features.knowledge && (
            <Badge variant="outline">knowledge</Badge>
          )}
          {!agent.user_callable && (
            <Badge variant="outline">hidden</Badge>
          )}
        </div>
      </div>
    </article>
  );
}
