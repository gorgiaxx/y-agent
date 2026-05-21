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
} from 'lucide-react';
import { Button } from '../ui';

import type { ModifiedFileEntry } from '../../hooks/useInfoPanel';
import type { PlanDisplayMeta, PlanTaskDisplay } from '../chat-panel/chat-box/planToolDisplay';
import type { LoopDisplayMeta } from '../chat-panel/chat-box/toolCallUtils';
import './InfoPanel.css';

function FileCard({ file }: { file: ModifiedFileEntry }) {
  return (
    <div className="info-file-card" title={file.filePath}>
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
  );
}

function derivePlanStatus(plan: PlanDisplayMeta): string {
  if (plan.kind === 'plan_stage') {
    return plan.stageStatus;
  }
  if (plan.failed > 0) return 'failed';
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

interface InfoPanelProps {
  modifiedFiles: ModifiedFileEntry[];
  planStatus: PlanDisplayMeta | null;
  loopStatus: LoopDisplayMeta | null;
  expanded: boolean;
  onToggleExpand: () => void;
  onClose: () => void;
}

export function InfoPanel({
  modifiedFiles,
  planStatus,
  loopStatus,
  expanded,
  onToggleExpand,
  onClose,
}: InfoPanelProps) {
  const [filesOpen, setFilesOpen] = useState(true);
  const [planOpen, setPlanOpen] = useState(true);
  const [loopOpen, setLoopOpen] = useState(true);

  const hasAny = modifiedFiles.length > 0 || planStatus !== null || loopStatus !== null;

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

            {planStatus && (
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
                  <span className="info-section-title">Plan</span>
                </div>
                {planOpen && <PlanCard plan={planStatus} />}
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
