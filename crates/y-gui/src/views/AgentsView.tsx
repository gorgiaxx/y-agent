import { AgentsPanel } from '../components/agents/AgentsPanel';
import { useAgentsContext, useNavigationContext } from '../providers/AppContexts';

export function AgentsView() {
  const agentHooks = useAgentsContext();
  const navProps = useNavigationContext();

  return (
    <AgentsPanel
      agentId={navProps.activeAgentId}
      onGetDetail={agentHooks.getAgentDetail}
      onSave={agentHooks.saveAgent}
      onReset={agentHooks.resetAgent}
      onReload={agentHooks.reloadAgents}
    />
  );
}
