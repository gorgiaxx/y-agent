import type { ReactNode } from 'react';
import { ArrowLeft, Loader2, RefreshCw, Square, SquareTerminal } from 'lucide-react';

import type { BackgroundTaskInfo, BackgroundTaskLogEntry } from '../../hooks/useBackgroundTasks';
import { formatDuration } from '../../utils/formatDuration';
import { NavDivider, NavItem, NavSidebar } from '../common/NavSidebar';
import { Button } from '../ui';
import { AnsiOutput } from './AnsiOutput';
import { outputContent } from './backgroundTaskOutput';
import './BackgroundTasksPanel.css';

interface BackgroundTasksSidebarPanelProps {
  tasks: BackgroundTaskInfo[];
  loading: boolean;
  error: string | null;
  selectedProcessId: string | null;
  onSelectTask: (processId: string) => void;
  onRefresh: () => void;
}

interface BackgroundTasksOutputPanelProps {
  task: BackgroundTaskInfo | null;
  logs: BackgroundTaskLogEntry[];
  busy: boolean;
  onPoll: (processId: string) => void;
  onKill: (processId: string) => void;
}

interface BackgroundTasksSidebarNavProps {
  children: ReactNode;
  onBack: () => void;
}

function statusLabel(task: BackgroundTaskInfo): string {
  if (task.status === 'completed' && task.exit_code != null) {
    return `completed ${task.exit_code}`;
  }
  return task.status;
}

function taskCountLabel(count: number): string {
  return `${count} task${count === 1 ? '' : 's'}`;
}

export function BackgroundTasksSidebarNav({
  children,
  onBack,
}: BackgroundTasksSidebarNavProps) {
  return (
    <NavSidebar>
      <NavItem
        icon={<ArrowLeft size={15} />}
        label="Back"
        onClick={onBack}
      />
      <NavDivider />
      <div className="sidebar-chat-region">
        {children}
      </div>
    </NavSidebar>
  );
}

export function BackgroundTasksSidebarPanel({
  tasks,
  loading,
  error,
  selectedProcessId,
  onSelectTask,
  onRefresh,
}: BackgroundTasksSidebarPanelProps) {
  const runningCount = tasks.filter((task) => task.status === 'running').length;

  return (
    <div className="background-tasks-sidebar">
      <div className="agent-session-toolbar">
        <div className="agent-session-toolbar-label">
          <SquareTerminal size={13} />
          <span>Background tasks</span>
          <span className="background-tasks-count">{taskCountLabel(tasks.length)}</span>
        </div>
        <div className="agent-session-toolbar-actions">
          <Button
            type="button"
            variant="icon"
            size="sm"
            onClick={onRefresh}
            disabled={loading}
            title="Refresh"
            aria-label="Refresh background tasks"
          >
            {loading ? <Loader2 size={13} className="background-tasks-spin" /> : <RefreshCw size={13} />}
          </Button>
        </div>
      </div>

      {runningCount > 0 && (
        <div className="background-tasks-sidebar-summary">{runningCount} running</div>
      )}

      {error && <div className="background-tasks-error">{error}</div>}

      <div className="background-tasks-sidebar-list">
        {tasks.length === 0 ? (
          <div className="background-tasks-empty">No background tasks</div>
        ) : (
          tasks.map((task) => (
            <button
              key={task.process_id}
              type="button"
              className={`background-tasks-sidebar-item ${
                selectedProcessId === task.process_id ? 'active' : ''
              }`}
              onClick={() => onSelectTask(task.process_id)}
            >
              <span className="background-tasks-sidebar-command" title={task.command}>
                {task.command}
              </span>
              <span className="background-tasks-sidebar-meta">
                <span className={`background-tasks-status background-tasks-status--${task.status}`}>
                  {statusLabel(task)}
                </span>
                <span>{formatDuration(task.duration_ms)}</span>
              </span>
            </button>
          ))
        )}
      </div>
    </div>
  );
}

export function BackgroundTasksOutputPanel({
  task,
  logs,
  busy,
  onPoll,
  onKill,
}: BackgroundTasksOutputPanelProps) {
  if (!task) {
    return (
      <div className="background-tasks-main-empty">
        <SquareTerminal size={28} />
        <span>Select a background task</span>
      </div>
    );
  }

  const isRunning = task.status === 'running';
  const content = outputContent(logs);

  return (
    <section className="background-tasks-main-panel">
      <header className="background-tasks-main-header">
        <div className="background-tasks-main-title-wrap">
          <div className="background-tasks-main-title" title={task.command}>{task.command}</div>
          <div className="background-tasks-main-meta">
            <span className={`background-tasks-status background-tasks-status--${task.status}`}>
              {statusLabel(task)}
            </span>
            <span>{task.backend}</span>
            <span>{formatDuration(task.duration_ms)}</span>
          </div>
        </div>
        <div className="background-tasks-main-actions">
          <Button
            type="button"
            variant="icon"
            size="sm"
            onClick={() => onPoll(task.process_id)}
            disabled={busy}
            title="Poll"
            aria-label={`Poll task ${task.process_id}`}
          >
            {busy ? <Loader2 size={14} className="background-tasks-spin" /> : <RefreshCw size={14} />}
          </Button>
          {isRunning && (
            <Button
              type="button"
              variant="icon"
              size="sm"
              className="background-tasks-danger-action"
              onClick={() => onKill(task.process_id)}
              disabled={busy}
              title="Kill"
              aria-label={`Kill task ${task.process_id}`}
            >
              <Square size={13} />
            </Button>
          )}
        </div>
      </header>

      {task.working_dir && (
        <div className="background-tasks-main-cwd" title={task.working_dir}>
          {task.working_dir}
        </div>
      )}

      {task.error && (
        <div className="background-tasks-error">{task.error}</div>
      )}

      <div className="background-task-console" aria-label="Task output">
        {content ? (
          <AnsiOutput content={content} />
        ) : (
          <div className="background-task-console-empty">No output yet</div>
        )}
      </div>
    </section>
  );
}
