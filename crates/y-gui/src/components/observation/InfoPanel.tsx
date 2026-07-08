import { useState } from 'react';
import {
  Info,
  X,
  Maximize2,
  Minimize2,
  ChevronDown,
  ChevronRight,
  FileEdit,
  FilePlus2,
  ClipboardList,
  Repeat,
  Boxes,
} from 'lucide-react';
import { Button } from '../ui';

import type { ModifiedFileEntry, ChildSessionSummary } from '../../hooks/useInfoPanel';
import { buildFileContextMenuItems } from '../chat-panel/chat-box/fileContextMenu';
import { useContextMenu } from '../chat-panel/chat-box/useContextMenu';
import { FileDiffView } from '../chat-panel/chat-box/tool-renderers/shared';
import type { PlanDisplayMeta, PlanTaskDisplay } from '../chat-panel/chat-box/planToolDisplay';
import type { LoopDisplayMeta } from '../chat-panel/chat-box/toolCallUtils';
import { platform } from '../../lib';
import './InfoPanel.css';

function FileCard({ file }: { file: ModifiedFileEntry }) {
  const contextMenu = useContextMenu();
  const canRevealInFileManager = platform.capabilities.revealFileManager;
  const hasDiffs = file.diffs.length > 0;

  const handleContextMenu = (event: React.MouseEvent) => {
    const items = buildFileContextMenuItems(file.filePath, {
      openFile: true,
      revealInFileManager: canRevealInFileManager,
      copyPath: true,
    });
    contextMenu.show(event, items);
  };

  return (
    <div className="info-file-block">
      <div
        className="info-file-card"
        title={file.filePath}
        data-file-context-menu="true"
        onContextMenu={handleContextMenu}
      >
        <div className={`info-file-icon type-${file.toolType}`}>
          {file.toolType === 'edit' ? <FileEdit size={12} /> : <FilePlus2 size={12} />}
        </div>
        <div className="info-file-details">
          <div className="info-file-name">{file.displayName}</div>
          <div className="info-file-path">{file.filePath}</div>
        </div>
        {file.count > 1 && (
          <span className="info-file-count">{file.count}x</span>
        )}
      </div>
      {hasDiffs && (
        <div className="info-file-diffs">
          {file.diffs.map((diff, index) => (
            <div
              key={`${file.filePath}-${index}-${diff.oldString.length}-${diff.newString.length}`}
              className="info-file-diff"
            >
              <div className="info-file-diff-label">Diff {index + 1}</div>
              <FileDiffView oldString={diff.oldString} newString={diff.newString} />
            </div>
          ))}
        </div>
      )}
      {contextMenu.rendered}
    </div>
  );
}

function mapPlanRunStatus(runStatus: string): string | null {
  switch (runStatus) {
    case 'completed': return 'completed';
    case 'partial_failure': return 'partial';
    case 'rejected': return 'rejected';
    case 'cancelled': return 'cancelled';
    case 'awaiting_approval': return 'awaiting';
    case 'running': return 'running';
    default: return null;
  }
}

function derivePlanStatus(plan: PlanDisplayMeta): string {
  if (plan.kind === 'plan_stage') {
    if (
      plan.reviewStatus === 'awaiting_user'
      || plan.reviewStatus === 'feedback_received'
    ) {
      return 'running';
    }
    return plan.stageStatus;
  }
  // History-sourced plans carry the authoritative persisted status, which is
  // the only reliable signal for terminal states (rejected/cancelled) whose
  // tasks may all read as pending.
  if (plan.planRunStatus) {
    const mapped = mapPlanRunStatus(plan.planRunStatus);
    if (mapped) return mapped;
  }
  if (plan.failed > 0) return 'failed';
  const tasks = plan.tasks;
  if (tasks.length > 0) {
    if (tasks.some((t) => t.status === 'in_progress' || t.status === 'pending')) {
      return 'running';
    }
    return 'completed';
  }
  if (plan.completed >= plan.totalPhases && plan.totalPhases > 0) return 'completed';
  return 'running';
}

function PlanCard({ plan }: { plan: PlanDisplayMeta }) {
  const status = derivePlanStatus(plan);
  const tasks: PlanTaskDisplay[] = plan.tasks;
  const total = tasks.length > 0
    ? tasks.length
    : (plan.kind === 'plan_execution' ? plan.totalPhases : 0);
  const completed = tasks.filter((t) => t.status === 'completed').length;
  const pct = total > 0 ? (completed / total) * 100 : 0;

  return (
    <div className="info-plan-card">
      <div className="info-plan-header">
        <div className="info-plan-icon">
          <ClipboardList size={12} />
        </div>
        <span className="info-plan-title">{plan.planTitle || 'Plan'}</span>
        <span className={`info-plan-status status-${status}`}>{status}</span>
      </div>
      <div className="info-progress-bar">
        <div className="info-progress-fill" style={{ width: `${Math.min(pct, 100)}%` }} />
      </div>
      {tasks.length > 0 && (
        <div className="info-task-list">
          {tasks.map((task) => (
            <div key={task.id || task.title} className="info-task-item">
              <span className={`info-task-dot status-${task.status}`} />
              <span className="info-task-title" title={task.description}>{task.title}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function LoopCard({ loop }: { loop: LoopDisplayMeta }) {
  if (loop.kind === 'loop_init') {
    return (
      <div className="info-loop-card">
        <div className="info-loop-header">
          <div className="info-loop-icon"><Repeat size={12} /></div>
          <span className="info-loop-round">Initializing</span>
          <span className="info-loop-status status-running">init</span>
        </div>
      </div>
    );
  }

  if (loop.kind === 'loop_round') {
    return (
      <div className="info-loop-card">
        <div className="info-loop-header">
          <div className="info-loop-icon"><Repeat size={12} /></div>
          <span className="info-loop-round">Round {loop.round} / {loop.maxRounds}</span>
          <span className={`info-loop-status ${loop.converged ? 'status-converged' : 'status-running'}`}>
            {loop.converged ? 'converged' : loop.roundStatus}
          </span>
        </div>
        {loop.tasksCompleted.length > 0 && (
          <div className="info-loop-tasks">
            <div className="info-loop-tasks-label">Completed</div>
            {loop.tasksCompleted.map((t) => (
              <div key={t} className="info-loop-task-item completed">{t}</div>
            ))}
          </div>
        )}
        {loop.tasksRemaining.length > 0 && (
          <div className="info-loop-tasks">
            <div className="info-loop-tasks-label">Remaining</div>
            {loop.tasksRemaining.map((t) => (
              <div key={t} className="info-loop-task-item remaining">{t}</div>
            ))}
          </div>
        )}
      </div>
    );
  }

  if (loop.kind === 'loop_review') {
    return (
      <div className="info-loop-card">
        <div className="info-loop-header">
          <div className="info-loop-icon"><Repeat size={12} /></div>
          <span className="info-loop-round">Review</span>
          <span className="info-loop-status status-running">{loop.reviewStatus}</span>
        </div>
      </div>
    );
  }

  // loop_execution
  if (loop.kind === 'loop_execution') {
    return (
      <div className="info-loop-card">
        <div className="info-loop-header">
          <div className="info-loop-icon"><Repeat size={12} /></div>
          <span className="info-loop-round">{loop.totalRounds} / {loop.maxRounds} rounds</span>
          <span className={`info-loop-status status-${loop.finalStatus === 'converged' ? 'converged' : 'completed'}`}>
            {loop.finalStatus}
          </span>
        </div>
      </div>
    );
  }

  return null;
}

/** Format a sub-agent's completion timestamp (RFC 3339) as HH:MM:SS for
  display in the info panel. Returns an empty string when the timestamp is
  missing or unparseable. */
function formatCompletionTime(iso: string): string {
  if (!iso) return '';
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

interface InfoPanelProps {
  modifiedFiles: ModifiedFileEntry[];
  plans: PlanDisplayMeta[];
  loopStatus: LoopDisplayMeta | null;
  childSessions: ChildSessionSummary[];
  onOpenChildSession?: (childSessionId: string, title: string) => void;
  expanded: boolean;
  onToggleExpand: () => void;
  onClose: () => void;
}

export function InfoPanel({
  modifiedFiles,
  plans,
  loopStatus,
  childSessions,
  onOpenChildSession,
  expanded,
  onToggleExpand,
  onClose,
}: InfoPanelProps) {
  const [filesOpen, setFilesOpen] = useState(true);
  const [planOpen, setPlanOpen] = useState(true);
  const [loopOpen, setLoopOpen] = useState(true);
  const [subOpen, setSubOpen] = useState(true);

  const hasAny = modifiedFiles.length > 0
    || plans.length > 0
    || loopStatus !== null
    || childSessions.length > 0;

  const panelContent = (
    <div className={`info-panel ${expanded ? 'info-expanded' : ''}`}>
      <div className="info-header">
        <div className="info-header-left">
          <Info size={16} className="info-header-icon" />
          <h3 className="info-title">Info</h3>
        </div>
        <div className="info-header-actions">
          <Button variant="icon" size="sm" onClick={onToggleExpand} title={expanded ? 'Collapse' : 'Expand'}>
            {expanded ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
          </Button>
          <Button variant="icon" size="sm" onClick={onClose} title="Close">
            <X size={14} />
          </Button>
        </div>
      </div>

      <div className="info-content">
        {!hasAny ? (
          <div className="info-empty">
            <Info size={28} className="info-empty-icon" />
            <p className="info-empty-text">No activity yet</p>
            <p className="info-empty-hint">
              Modified files, plan progress, and loop status will appear here during a session.
            </p>
          </div>
        ) : (
          <>
            {modifiedFiles.length > 0 && (
              <div className="info-section">
                <div
                  className="info-section-header"
                  onClick={() => setFilesOpen(!filesOpen)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') setFilesOpen(!filesOpen); }}
                >
                  <span className="info-section-chevron">
                    {filesOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  </span>
                  <span className="info-section-title">Modified Files</span>
                  <span className="info-section-count">{modifiedFiles.length}</span>
                </div>
                {filesOpen && modifiedFiles.map((f) => (
                  <FileCard key={f.filePath} file={f} />
                ))}
              </div>
            )}

            {plans.length > 0 && (
              <div className="info-section">
                <div
                  className="info-section-header"
                  onClick={() => setPlanOpen(!planOpen)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') setPlanOpen(!planOpen); }}
                >
                  <span className="info-section-chevron">
                    {planOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  </span>
                  <span className="info-section-title">
                    {plans.length > 1 ? `Plans (${plans.length})` : 'Plan'}
                  </span>
                </div>
                {planOpen && plans.map((plan, idx) => (
                  <PlanCard key={plan.planFile || plan.planTitle || `plan-${idx}`} plan={plan} />
                ))}
              </div>
            )}

            {loopStatus && (
              <div className="info-section">
                <div
                  className="info-section-header"
                  onClick={() => setLoopOpen(!loopOpen)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') setLoopOpen(!loopOpen); }}
                >
                  <span className="info-section-chevron">
                    {loopOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  </span>
                  <span className="info-section-title">Loop</span>
                </div>
                {loopOpen && <LoopCard loop={loopStatus} />}
              </div>
            )}

            {childSessions.length > 0 && (
              <div className="info-section">
                <div
                  className="info-section-header"
                  onClick={() => setSubOpen(!subOpen)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') setSubOpen(!subOpen); }}
                >
                  <span className="info-section-chevron">
                    {subOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  </span>
                  <span className="info-section-title">Sub-agents ({childSessions.length})</span>
                </div>
                {subOpen && (
                  <div className="info-subagent-list">
                    {childSessions.map((child) => (
                      <button
                        key={child.id}
                        type="button"
                        className="info-subagent-item"
                        title={child.title ?? child.id}
                        onClick={() => onOpenChildSession?.(child.id, child.title ?? 'Sub-agent')}
                      >
                        <Boxes size={12} className="info-subagent-icon" />
                        <span className="info-subagent-title">
                          {child.title ?? child.agentId ?? 'Sub-agent'}
                        </span>
                        <span className={`info-subagent-status status-${child.status}`}>
                          {child.status === 'running'
                            ? 'running'
                            : formatCompletionTime(child.updatedAt) || 'done'}
                        </span>
                        <ChevronRight size={12} className="info-subagent-chevron" />
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );

  if (expanded) {
    return (
      <div className="info-backdrop" onClick={onClose}>
        <div onClick={(e) => e.stopPropagation()}>
          {panelContent}
        </div>
      </div>
    );
  }

  return panelContent;
}
