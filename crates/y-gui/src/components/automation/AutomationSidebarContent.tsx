import { useAutomationContext, useAutomationNavContext } from '../../providers/AppContexts';
import { AutomationSidebarPanel } from './AutomationSidebarPanel';

export function AutomationSidebarContent() {
  const autoHooks = useAutomationContext();
  const navProps = useAutomationNavContext();

  return (
    <AutomationSidebarPanel
      workflows={autoHooks.workflows}
      schedules={autoHooks.schedules}
      selectedType={navProps.automationSelectedType}
      selectedId={navProps.automationSelectedId}
      onSelectWorkflow={(id: string) => {
        navProps.setAutomationSelectedType('workflow');
        navProps.setAutomationSelectedId(id);
        navProps.setAutomationCreating(null);
      }}
      onSelectSchedule={(id: string) => {
        navProps.setAutomationSelectedType('schedule');
        navProps.setAutomationSelectedId(id);
        navProps.setAutomationCreating(null);
      }}
      onCreateWorkflow={() => {
        navProps.setAutomationSelectedType(null);
        navProps.setAutomationSelectedId(null);
        navProps.setAutomationCreating('workflow');
      }}
      onCreateSchedule={() => {
        navProps.setAutomationSelectedType(null);
        navProps.setAutomationSelectedId(null);
        navProps.setAutomationCreating('schedule');
      }}
    />
  );
}
