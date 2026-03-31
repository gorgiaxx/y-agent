/**
 * AutomationSidebarPanel -- sidebar panel for workflows and schedules.
 *
 * Shows two collapsible sections: Workflows and Schedules. Each section
 * lists items with badges and supports selection and creation.
 */
import { useState } from 'react';
import { ChevronRight, ChevronDown, Plus, GitBranch, Clock } from 'lucide-react';
import type { WorkflowInfo, ScheduleInfo } from '../../hooks/useAutomation';
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

  /** Parse tags from JSON string or return empty array. */
  const parseTags = (tagsStr: string): string[] => {
    try {
      const arr = JSON.parse(tagsStr);
      return Array.isArray(arr) ? arr : [];
    } catch {
      return [];
    }
  };

  return (
    <div className="automation-sidebar">
      {/* Workflows section */}
      <div className="automation-sidebar-section">
        <button
          className="automation-sidebar-section-header"
          onClick={() => setWorkflowsOpen(!workflowsOpen)}
        >
          {workflowsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          <GitBranch size={14} />
          <span className="automation-sidebar-section-title">Workflows</span>
          <span className="automation-sidebar-count">{workflows.length}</span>
          <button
            className="automation-sidebar-add-btn"
            onClick={(e) => {
              e.stopPropagation();
              onCreateWorkflow();
            }}
            title="Create Workflow"
          >
            <Plus size={14} />
          </button>
        </button>
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
                      <span className={`automation-sidebar-badge automation-sidebar-badge--${wf.format === 'expression_dsl' ? 'dsl' : 'toml'}`}>
                        {wf.format === 'expression_dsl' ? 'DSL' : 'TOML'}
                      </span>
                    </div>
                    {tags.length > 0 && (
                      <div className="automation-sidebar-item-tags">
                        {tags.slice(0, 3).map((t) => (
                          <span key={t} className="automation-sidebar-tag">{t}</span>
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
        <button
          className="automation-sidebar-section-header"
          onClick={() => setSchedulesOpen(!schedulesOpen)}
        >
          {schedulesOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          <Clock size={14} />
          <span className="automation-sidebar-section-title">Schedules</span>
          <span className="automation-sidebar-count">{schedules.length}</span>
          <button
            className="automation-sidebar-add-btn"
            onClick={(e) => {
              e.stopPropagation();
              onCreateSchedule();
            }}
            title="Create Schedule"
          >
            <Plus size={14} />
          </button>
        </button>
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
                    <span className={`automation-sidebar-badge automation-sidebar-badge--${sc.trigger_type}`}>
                      {sc.trigger_type}
                    </span>
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
