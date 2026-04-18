import { AutomationPanel } from '../components/automation/AutomationPanel';
import { useAutomationContext, useNavigationContext } from '../providers/AppContexts';

export function AutomationView() {
  const autoHooks = useAutomationContext();
  const navProps = useNavigationContext();

  return (
    <AutomationPanel
      selectedType={navProps.automationSelectedType}
      selectedId={navProps.automationSelectedId}
      getWorkflow={autoHooks.getWorkflow}
      createWorkflow={autoHooks.createWorkflow}
      updateWorkflow={autoHooks.updateWorkflow}
      deleteWorkflow={autoHooks.deleteWorkflow}
      validateWorkflow={autoHooks.validateWorkflow}
      getWorkflowDag={autoHooks.getWorkflowDag}
      schedules={autoHooks.schedules}
      workflows={autoHooks.workflows}
      getSchedule={autoHooks.getSchedule}
      createSchedule={autoHooks.createSchedule}
      updateSchedule={autoHooks.updateSchedule}
      deleteSchedule={autoHooks.deleteSchedule}
      pauseSchedule={autoHooks.pauseSchedule}
      resumeSchedule={autoHooks.resumeSchedule}
      getExecutionHistory={autoHooks.getExecutionHistory}
      triggerScheduleNow={autoHooks.triggerScheduleNow}
      executeWorkflow={autoHooks.executeWorkflow}
      isCreating={navProps.automationCreating}
      onCancelCreate={() => navProps.setAutomationCreating(null)}
    />
  );
}
