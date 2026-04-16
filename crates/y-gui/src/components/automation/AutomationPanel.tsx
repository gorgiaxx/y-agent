/**
 * AutomationPanel -- main content panel for the Automation view.
 *
 * Shows workflow or schedule detail based on sidebar selection.
 * Supports creating, editing, deleting workflows and schedules.
 * Renders DAG visualization for workflows.
 */
import { Zap } from 'lucide-react';
import { WorkflowDetail } from './WorkflowDetail';
import { ScheduleDetail } from './ScheduleDetail';
import { WorkflowCreateForm, ScheduleCreateForm } from './CreateForms';
import type { AutomationPanelProps } from './types';
import './AutomationPanel.css';

export function AutomationPanel({
  selectedType,
  selectedId,
  getWorkflow,
  createWorkflow,
  updateWorkflow,
  deleteWorkflow,
  validateWorkflow,
  getWorkflowDag,

  workflows,
  getSchedule,
  createSchedule,
  updateSchedule,
  deleteSchedule,
  pauseSchedule,
  resumeSchedule,
  getExecutionHistory,
  triggerScheduleNow,
  executeWorkflow,
  isCreating,
  onCancelCreate,
}: AutomationPanelProps) {
  // -- Empty state --
  if (!selectedId && !isCreating) {
    return (
      <div className="automation-panel">
        <div className="automation-empty">
          <Zap size={40} className="automation-empty-icon" />
          <p className="automation-empty-title">Automation</p>
          <p className="automation-empty-desc">
            Create and manage workflows and scheduled tasks.
            Select an item from the sidebar or create a new one.
          </p>
        </div>
      </div>
    );
  }

  if (isCreating === 'workflow') {
    return (
      <WorkflowCreateForm
        onSave={createWorkflow}
        onValidate={validateWorkflow}
        onCancel={onCancelCreate}
      />
    );
  }

  if (isCreating === 'schedule') {
    return (
      <ScheduleCreateForm
        workflows={workflows}
        onSave={createSchedule}
        onCancel={onCancelCreate}
      />
    );
  }

  if (selectedType === 'workflow' && selectedId) {
    return (
      <WorkflowDetail
        id={selectedId}
        getWorkflow={getWorkflow}
        updateWorkflow={updateWorkflow}
        deleteWorkflow={deleteWorkflow}
        validateWorkflow={validateWorkflow}
        getWorkflowDag={getWorkflowDag}
        executeWorkflow={executeWorkflow}
        getExecutionHistory={getExecutionHistory}
      />
    );
  }

  if (selectedType === 'schedule' && selectedId) {
    return (
      <ScheduleDetail
        id={selectedId}
        workflows={workflows}
        getSchedule={getSchedule}
        updateSchedule={updateSchedule}
        deleteSchedule={deleteSchedule}
        pauseSchedule={pauseSchedule}
        resumeSchedule={resumeSchedule}
        getExecutionHistory={getExecutionHistory}
        triggerScheduleNow={triggerScheduleNow}
      />
    );
  }

  return null;
}
