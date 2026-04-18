/**
 * AutomationSidebarPanel -- sidebar panel for workflows and schedules.
 *
 * Shows two collapsible sections: Workflows and Schedules. Each section
 * lists items with badges and supports selection and creation.
 */
import { useState } from 'react';
import { ChevronRight, ChevronDown, Plus } from 'lucide-react';
import type { WorkflowInfo, ScheduleInfo } from '../../hooks/useAutomation';
import { Badge } from '../ui';
import { Button } from '../ui/Button';
import { parseTags } from '../../utils/parseTags';
import './AutomationPanel.css';

interface AutomationSidebarPanelProps {
  workflows: WorkflowInfo[];
  schedules: ScheduleInfo[];
  selectedType: 'workflow' | 'schedule' | null;
  selectedId: string | null;
  onSelectWorkflow: (id: string) => void;
  onSelectSchedule: (id: string) => void;
  onCreateWorkflow: () => void;
  onCreateSchedule: () => void;
}

export function AutomationSidebarPanel({
  workflows,
  schedules,
  selectedType,
  selectedId,
  onSelectWorkflow,
  onSelectSchedule,
  onCreateWorkflow,
  onCreateSchedule,
}: AutomationSidebarPanelProps) {
  const [workflowsOpen, setWorkflowsOpen] = useState(true);
  const [schedulesOpen, setSchedulesOpen] = useState(true);

  return (
    <div className="automation-sidebar">
      {/* Workflows section */}
      <div className="automation-sidebar-section">
        <div className="agent-session-toolbar">
          <div
            className="agent-session-toolbar-label agent-session-toolbar-label--toggle"
            onClick={() => setWorkflowsOpen(!workflowsOpen)}
          >
            {workflowsOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            <span>Workflows</span>
            <div className="agent-session-toolbar-meta">
              <Badge variant="outline">{workflows.length}</Badge>
            </div>
          </div>
          <div className="agent-session-toolbar-actions">
            <Button
              variant="icon"
              size="sm"
              onClick={onCreateWorkflow}
              title="Create Workflow"
            >
              <Plus size={14} />
            </Button>
          </div>
        </div>
        {workflowsOpen && (
          <div className="automation-sidebar-list">
            {workflows.length === 0 ? (
              <div className="automation-sidebar-empty">No workflows yet</div>
            ) : (
              workflows.map((wf) => {
                const tags = parseTags(wf.tags);
                return (
                  <button
                    key={wf.id}
                    className={`automation-sidebar-item ${
                      selectedType === 'workflow' && selectedId === wf.id ? 'active' : ''
                    }`}
                    onClick={() => onSelectWorkflow(wf.id)}
                  >
                    <div className="automation-sidebar-item-main">
                      <span className="automation-sidebar-item-name">{wf.name}</span>
                      <Badge variant={wf.format === 'expression_dsl' ? 'info' : 'accent'}>
                        {wf.format === 'expression_dsl' ? 'DSL' : 'TOML'}
                      </Badge>
                    </div>
                    {tags.length > 0 && (
                      <div className="automation-sidebar-item-tags">
                        {tags.slice(0, 3).map((t) => (
                          <Badge key={t} variant="outline">{t}</Badge>
                        ))}
                      </div>
                    )}
                  </button>
                );
              })
            )}
          </div>
        )}
      </div>

      {/* Schedules section */}
      <div className="automation-sidebar-section">
        <div className="agent-session-toolbar">
          <div
            className="agent-session-toolbar-label agent-session-toolbar-label--toggle"
            onClick={() => setSchedulesOpen(!schedulesOpen)}
          >
            {schedulesOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            <span>Schedules</span>
            <div className="agent-session-toolbar-meta">
              <Badge variant="outline">{schedules.length}</Badge>
            </div>
          </div>
          <div className="agent-session-toolbar-actions">
            <Button
              variant="icon"
              size="sm"
              onClick={onCreateSchedule}
              title="Create Schedule"
            >
              <Plus size={14} />
            </Button>
          </div>
        </div>
        {schedulesOpen && (
          <div className="automation-sidebar-list">
            {schedules.length === 0 ? (
              <div className="automation-sidebar-empty">No schedules yet</div>
            ) : (
              schedules.map((sc) => (
                <button
                  key={sc.id}
                  className={`automation-sidebar-item ${
                    selectedType === 'schedule' && selectedId === sc.id ? 'active' : ''
                  }`}
                  onClick={() => onSelectSchedule(sc.id)}
                >
                  <div className="automation-sidebar-item-main">
                    <span className="automation-sidebar-item-name">{sc.name}</span>
                    <Badge variant="accent">
                      {sc.trigger_type}
                    </Badge>
                    <span className={`automation-sidebar-status ${sc.enabled ? 'active' : 'paused'}`}>
                      {sc.enabled ? '' : '||'}
                    </span>
                  </div>
                </button>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}
