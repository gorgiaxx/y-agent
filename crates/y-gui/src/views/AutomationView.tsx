import { AutomationPanel } from '../components/automation/AutomationPanel';
import { AutomationSidebarPanel } from '../components/automation/AutomationSidebarPanel';
import { NavSidebar } from '../components/common/NavSidebar';
import { useAutomationContext, useNavigationContext } from '../providers/AppContexts';

export function AutomationView() {
  const autoHooks = useAutomationContext();
  const navProps = useNavigationContext();

  return (
    <div className="view-shell">
      <NavSidebar bare>
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
      </NavSidebar>

      <section className="view-main-pane">
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
      </section>
    </div>
  );
}
