import { Bot } from 'lucide-react';

interface AgentInfo {
  id: string;
  name: string;
  description: string;
  mode: string;
  trust_tier: string;
  is_overridden: boolean;
}

interface AgentsSidebarPanelProps {
  agents: AgentInfo[];
  activeAgentId: string | null;
  onSelectAgent: (id: string) => void;
}

const TIER_LABELS: Record<string, string> = {
  BuiltIn: 'Built-in',
  UserDefined: 'User-Defined',
  Dynamic: 'Dynamic',
};

const TIERS = ['BuiltIn', 'UserDefined', 'Dynamic'] as const;

export function AgentsSidebarPanel({
  agents,
  activeAgentId,
  onSelectAgent,
}: AgentsSidebarPanelProps) {
  if (agents.length === 0) {
    return (
      <div className="sidebar-list">
        <div className="session-empty">No agents registered</div>
      </div>
    );
  }

  return (
    <div className="sidebar-list">
      {TIERS.map((tier) => {
        const tierAgents = agents.filter((a) => a.trust_tier === tier);
        if (tierAgents.length === 0) return null;
        return (
          <div key={tier}>
            <div className="workspace-label workspace-label--general">
              <span className="workspace-name">{TIER_LABELS[tier]} ({tierAgents.length})</span>
            </div>
            {tierAgents.map((agent) => (
              <div
                key={agent.id}
                className={`sidebar-item ${activeAgentId === agent.id ? 'active' : ''}`}
                onClick={() => onSelectAgent(agent.id)}
              >
                <div className="sidebar-item-header">
                  <Bot size={14} className="sidebar-item-icon" />
                  <span className="sidebar-item-name">{agent.name}</span>
                  {agent.is_overridden && (
                    <span className="sidebar-item-badge" style={{ color: 'var(--warning)' }}>OVR</span>
                  )}
                </div>
                <p className="sidebar-item-desc">{agent.description}</p>
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}
