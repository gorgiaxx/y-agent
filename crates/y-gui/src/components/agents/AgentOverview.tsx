import { SquarePen } from 'lucide-react';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { ScrollArea } from '../ui/ScrollArea';
import type { AgentInfo } from '../../hooks/useAgents';
import {
  AgentGlyph,
  formatAgentModeLabel,
  formatAgentTierLabel,
  getAgentModeBadgeVariant,
} from './agentDisplay';

interface AgentOverviewProps {
  filteredAgents: AgentInfo[];
  agentQuery: string;
  onSelectAgent: (id: string) => void;
  onOpenEdit: (id: string) => void;
}

export function AgentOverview({
  filteredAgents,
  agentQuery,
  onSelectAgent,
  onOpenEdit,
}: AgentOverviewProps) {
  return (
    <ScrollArea className="flex-1 min-h-0">
      <div className="agents-overview">
        <section className="agents-gallery-section">
          <div className="agents-gallery-header">
            <div>
              <div className="agents-section-kicker">
                {agentQuery.trim() ? 'Filtered Results' : 'Preset Catalog'}
              </div>
              <h3 className="agents-gallery-title">
                {agentQuery.trim()
                  ? `${filteredAgents.length} matching presets`
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
          <AgentGlyph icon={agent.icon} name={agent.name} size={16} />
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
