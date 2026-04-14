/**
 * AutomationPanel -- main content panel for the Automation view.
 *
 * Shows workflow or schedule detail based on sidebar selection.
 * Supports creating, editing, deleting workflows and schedules.
 * Renders DAG visualization for workflows.
 */
import { useState, useEffect, useCallback } from 'react';
import {
  Zap,
  GitBranch,
  Clock,
  Pencil,
  Trash2,
  Save,
  X,
  Play,
  Pause,
  CheckCircle,
  AlertCircle,
  ChevronDown,
  ChevronRight,
  Copy,
  RotateCw,
} from 'lucide-react';
import {
  Button,
  Input,
  Textarea,
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '../ui';
import { DagGraph } from './DagGraph';
import type {
  WorkflowInfo,
  ScheduleInfo,
  DagVisualization,
  ValidationResult,
  ExecutionRecord,
} from '../../hooks/useAutomation';
import './AutomationPanel.css';

interface AutomationPanelProps {
  selectedType: 'workflow' | 'schedule' | null;
  selectedId: string | null;
  // Workflow ops
  getWorkflow: (id: string) => Promise<WorkflowInfo | null>;
  createWorkflow: (
    name: string,
    definition: string,
    format: string,
    description?: string,
    tags?: string,
  ) => Promise<WorkflowInfo | null>;
  updateWorkflow: (
    id: string,
    definition?: string,
    format?: string,
    description?: string,
    tags?: string,
  ) => Promise<WorkflowInfo | null>;
  deleteWorkflow: (id: string) => Promise<boolean>;
  validateWorkflow: (definition: string, format: string) => Promise<ValidationResult | null>;
  getWorkflowDag: (id: string) => Promise<DagVisualization | null>;
  // Schedule ops
  schedules: ScheduleInfo[];
  workflows: WorkflowInfo[];
  getSchedule: (id: string) => Promise<ScheduleInfo | null>;
  createSchedule: (request: {
    name: string;
    trigger: unknown;
    workflow_id: string;
    description?: string;
  }) => Promise<ScheduleInfo | null>;
  updateSchedule: (id: string, request: {
    name?: string;
    trigger?: unknown;
    workflow_id?: string;
    description?: string;
  }) => Promise<ScheduleInfo | null>;
  deleteSchedule: (id: string) => Promise<boolean>;
  pauseSchedule: (id: string) => Promise<boolean>;
  resumeSchedule: (id: string) => Promise<boolean>;
  // Execution history & replay
  getExecutionHistory: (scheduleId: string) => Promise<ExecutionRecord[]>;
  triggerScheduleNow: (scheduleId: string) => Promise<ExecutionRecord | null>;
  executeWorkflow: (workflowId: string) => Promise<ExecutionRecord | null>;
  // Creating new
  isCreating: 'workflow' | 'schedule' | null;
  onCancelCreate: () => void;
}

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

// ---------------------------------------------------------------------------
// WorkflowDetail
// ---------------------------------------------------------------------------

function WorkflowDetail({
  id,
  getWorkflow,
  updateWorkflow,
  deleteWorkflow,
  validateWorkflow,
  getWorkflowDag,
  executeWorkflow,
  getExecutionHistory,
}: {
  id: string;
  getWorkflow: (id: string) => Promise<WorkflowInfo | null>;
  updateWorkflow: (id: string, definition?: string, format?: string, description?: string, tags?: string) => Promise<WorkflowInfo | null>;
  deleteWorkflow: (id: string) => Promise<boolean>;
  validateWorkflow: (definition: string, format: string) => Promise<ValidationResult | null>;
  getWorkflowDag: (id: string) => Promise<DagVisualization | null>;
  executeWorkflow: (workflowId: string) => Promise<ExecutionRecord | null>;
  getExecutionHistory: (scheduleId: string) => Promise<ExecutionRecord[]>;
}) {
  const [wf, setWf] = useState<WorkflowInfo | null>(null);
  const [dag, setDag] = useState<DagVisualization | null>(null);
  const [editing, setEditing] = useState(false);
  const [editDef, setEditDef] = useState('');
  const [editDesc, setEditDesc] = useState('');
  const [validation, setValidation] = useState<ValidationResult | null>(null);
  const [saving, setSaving] = useState(false);

  // Execution state
  const [running, setRunning] = useState(false);
  const [lastResult, setLastResult] = useState<ExecutionRecord | null>(null);
  const [execHistory, setExecHistory] = useState<ExecutionRecord[]>([]);

  const loadHistory = useCallback(async () => {
    // Backend stores workflow executions with schedule_id = "workflow-{id}"
    const history = await getExecutionHistory(`workflow-${id}`);
    setExecHistory(history);
  }, [id, getExecutionHistory]);

  const load = useCallback(async () => {
    const w = await getWorkflow(id);
    setWf(w);
    if (w) {
      const d = await getWorkflowDag(id);
      setDag(d);
    }
    await loadHistory();
  }, [id, getWorkflow, getWorkflowDag, loadHistory]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    load();
    setEditing(false);
    setLastResult(null);
  }, [load]);

  const handleEdit = () => {
    if (!wf) return;
    setEditDef(wf.definition);
    setEditDesc(wf.description ?? '');
    setValidation(null);
    setEditing(true);
  };

  const handleValidate = async () => {
    if (!wf) return;
    const result = await validateWorkflow(editDef, wf.format);
    setValidation(result);
  };

  const handleSave = async () => {
    if (!wf) return;
    setSaving(true);
    await updateWorkflow(id, editDef, undefined, editDesc);
    setSaving(false);
    setEditing(false);
    load();
  };

  const handleDelete = async () => {
    const ok = await deleteWorkflow(id);
    if (ok) {
      setWf(null);
    }
  };

  const handleRun = async () => {
    setRunning(true);
    setLastResult(null);
    const result = await executeWorkflow(id);
    setRunning(false);
    setLastResult(result);
    // Refresh history to include the new execution.
    await loadHistory();
    // Auto-dismiss the result toast after 5 seconds.
    setTimeout(() => setLastResult(null), 5000);
  };

  if (!wf) {
    return <div className="automation-panel"><div className="automation-loading">Loading workflow...</div></div>;
  }

  const tags = (() => {
    try {
      const arr = JSON.parse(wf.tags);
      return Array.isArray(arr) ? arr : [];
    } catch {
      return [];
    }
  })();

  return (
    <div className="automation-panel">
      <div className="automation-detail">
        {/* Header */}
        <div className="automation-detail-header">
          <div className="automation-detail-info">
            <div className="automation-detail-title-row">
              <GitBranch size={20} className="automation-detail-icon" />
              <h2 className="automation-detail-name">{wf.name}</h2>
            </div>
            {wf.description && (
              <p className="automation-detail-desc">{wf.description}</p>
            )}
            <div className="automation-detail-badges">
              <span className={`automation-badge automation-badge--${wf.format === 'expression_dsl' ? 'dsl' : 'toml'}`}>
                {wf.format === 'expression_dsl' ? 'Expression DSL' : 'TOML'}
              </span>
              <span className="automation-badge automation-badge--creator">{wf.creator}</span>
              {tags.map((t: string) => (
                <span key={t} className="automation-badge automation-badge--tag">{t}</span>
              ))}
            </div>
          </div>
          <div className="automation-detail-actions">
            {!editing && (
              <>
                <button
                  className="automation-btn automation-btn--primary"
                  onClick={handleRun}
                  disabled={running}
                >
                  {running ? (
                    <><RotateCw size={14} className="spin-animation" /> Running...</>
                  ) : (
                    <><Play size={14} /> Run</>
                  )}
                </button>
                <button className="automation-btn" onClick={handleEdit}>
                  <Pencil size={14} /> Edit
                </button>
                <button className="automation-btn automation-btn--danger" onClick={handleDelete}>
                  <Trash2 size={14} /> Delete
                </button>
              </>
            )}
          </div>
        </div>

        {/* Execution result toast */}
        {lastResult && (
          <div className={`exec-result-toast exec-result-toast--${lastResult.status}`}>
            {lastResult.status === 'completed' ? (
              <><CheckCircle size={16} /> Workflow executed successfully</>
            ) : lastResult.status === 'failed' ? (
              <><AlertCircle size={16} /> Execution failed: {lastResult.error_message ?? 'Unknown error'}</>
            ) : (
              <><RotateCw size={16} /> Status: {lastResult.status}</>
            )}
            {lastResult.duration_ms !== null && (
              <span className="exec-result-duration">
                {lastResult.duration_ms < 1000
                  ? `${lastResult.duration_ms}ms`
                  : `${(lastResult.duration_ms / 1000).toFixed(1)}s`}
              </span>
            )}
          </div>
        )}

        {/* Editor */}
        {editing ? (
          <WorkflowEditor
            format={wf.format}
            editDef={editDef}
            editDesc={editDesc}
            onDefChange={setEditDef}
            onDescChange={setEditDesc}
            validation={validation}
            saving={saving}
            onValidate={handleValidate}
            onSave={handleSave}
            onCancel={() => setEditing(false)}
          />
        ) : (
          <>
            {/* Definition preview */}
            <div className="automation-section">
              <div className="automation-section-title">Definition</div>
              <pre className="automation-code">{wf.definition}</pre>
            </div>

            {/* DAG visualization */}
            {dag && dag.nodes.length > 0 && (
              <div className="automation-section">
                <div className="automation-section-title">DAG Visualization</div>
                <div className="automation-dag-container">
                  <DagGraph dag={dag} />
                </div>
              </div>
            )}

            {/* Metadata */}
            <div className="automation-section">
              <div className="automation-section-title">Details</div>
              <div className="automation-meta-grid">
                <div className="automation-meta-item">
                  <span className="automation-meta-label">Created</span>
                  <span className="automation-meta-value">{new Date(wf.created_at).toLocaleString()}</span>
                </div>
                <div className="automation-meta-item">
                  <span className="automation-meta-label">Updated</span>
                  <span className="automation-meta-value">{new Date(wf.updated_at).toLocaleString()}</span>
                </div>
                <div className="automation-meta-item">
                  <span className="automation-meta-label">ID</span>
                  <span className="automation-meta-value automation-meta-mono">{wf.id}</span>
                </div>
              </div>
            </div>

            {/* Execution History */}
            <div className="automation-section">
              <div className="automation-section-title">Execution History</div>
              <ExecutionHistorySection executions={execHistory} />
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// WorkflowEditor (structured TOML task cards + source tab)
// ---------------------------------------------------------------------------

/** Lightweight parsed task from TOML definition. */
interface ParsedTask {
  id: string;
  name: string;
  type: string;
  tool_name: string;
  depends_on: string;
  retry: string;
  failure_strategy: string;
  parameters: string;
}

/** Try to parse TOML definition into task objects for structured editing. */
function parseTomlTasks(def: string): ParsedTask[] | null {
  try {
    // Simple regex-based parser for [[workflow.tasks]] or [[tasks]] blocks.
    // We split by task array headers and extract key = value pairs.
    const taskBlocks: string[] = [];
    const taskPattern = /\[\[(workflow\.)?tasks\]\]/g;
    const matches = [...def.matchAll(taskPattern)];
    if (matches.length === 0) return null;

    for (let i = 0; i < matches.length; i++) {
      const start = matches[i].index! + matches[i][0].length;
      const end = i + 1 < matches.length ? matches[i + 1].index! : def.length;
      taskBlocks.push(def.slice(start, end).trim());
    }

    return taskBlocks.map((block) => {
      const get = (key: string): string => {
        // Match key = "value" or key = value or key = number
        const m = block.match(new RegExp(`^${key}\\s*=\\s*(.+)$`, 'm'));
        if (!m) return '';
        let val = m[1].trim();
        // Strip quotes
        if ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) {
          val = val.slice(1, -1);
        }
        return val;
      };

      // Extract [*.parameters] or [*.inputs] sub-table
      const paramBlock = block.match(/\[([\w.]*(?:parameters|inputs))\]\s*\n([\s\S]*?)(?=\n\[|$)/);
      let params = '';
      if (paramBlock) {
        params = paramBlock[2].trim();
      }

      return {
        id: get('id'),
        name: get('name'),
        type: get('type'),
        tool_name: get('tool_name') || get('tool'),
        depends_on: get('depends_on'),
        retry: get('retry') || get('max_retries'),
        failure_strategy: get('failure_strategy'),
        parameters: params,
      };
    });
  } catch {
    return null;
  }
}

/** Serialize parsed tasks back to TOML string. */
function serializeTasksToToml(tasks: ParsedTask[], originalDef: string): string {
  // Extract the [workflow] header (everything before the first [[...tasks]])
  const headerMatch = originalDef.match(/^([\s\S]*?)(?=\[\[(workflow\.)?tasks\]\])/);
  const header = headerMatch ? headerMatch[1].trim() : '[workflow]\nname = "workflow"';

  const taskBlocks = tasks.map((t) => {
    const lines: string[] = ['[[workflow.tasks]]'];
    if (t.id) lines.push(`id = "${t.id}"`);
    if (t.name) lines.push(`name = "${t.name}"`);
    if (t.type) lines.push(`type = "${t.type}"`);
    if (t.tool_name) lines.push(`tool_name = "${t.tool_name}"`);
    if (t.depends_on) lines.push(`depends_on = ${t.depends_on.startsWith('[') ? t.depends_on : `["${t.depends_on}"]`}`);
    if (t.retry) lines.push(`retry = ${t.retry}`);
    if (t.failure_strategy) lines.push(`failure_strategy = "${t.failure_strategy}"`);
    if (t.parameters) {
      lines.push(`[workflow.tasks.parameters]`);
      lines.push(t.parameters);
    }
    return lines.join('\n');
  });

  return header + '\n\n' + taskBlocks.join('\n\n') + '\n';
}

function WorkflowEditor({
  format,
  editDef,
  editDesc,
  onDefChange,
  onDescChange,
  validation,
  saving,
  onValidate,
  onSave,
  onCancel,
}: {
  format: string;
  editDef: string;
  editDesc: string;
  onDefChange: (v: string) => void;
  onDescChange: (v: string) => void;
  validation: ValidationResult | null;
  saving: boolean;
  onValidate: () => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  const isToml = format === 'toml';
  const [tab, setTab] = useState<'structured' | 'source'>(isToml ? 'structured' : 'source');
  const [tasks, setTasks] = useState<ParsedTask[]>([]);
  const [parseError, setParseError] = useState<string | null>(null);

  // Parse TOML when switching to structured tab or when editDef changes
  useEffect(() => {
    if (tab === 'structured' && isToml) {
      const parsed = parseTomlTasks(editDef);
      if (parsed) {
        // eslint-disable-next-line react-hooks/set-state-in-effect
        setTasks(parsed);
        setParseError(null);
      } else {
        setTasks([]);
        setParseError('Could not parse TOML tasks. Use the Source tab to edit manually.');
      }
    }
  }, [tab, isToml, editDef]);

  const handleTaskChange = (index: number, field: keyof ParsedTask, value: string) => {
    const updated = [...tasks];
    updated[index] = { ...updated[index], [field]: value };
    setTasks(updated);
    // Serialize back to TOML
    onDefChange(serializeTasksToToml(updated, editDef));
  };

  const handleAddTask = () => {
    const newTask: ParsedTask = {
      id: `step${tasks.length + 1}`,
      name: `Step ${tasks.length + 1}`,
      type: 'tool_execution',
      tool_name: '',
      depends_on: '',
      retry: '',
      failure_strategy: '',
      parameters: '',
    };
    const updated = [...tasks, newTask];
    setTasks(updated);
    onDefChange(serializeTasksToToml(updated, editDef));
  };

  const handleRemoveTask = (index: number) => {
    const updated = tasks.filter((_, i) => i !== index);
    setTasks(updated);
    onDefChange(serializeTasksToToml(updated, editDef));
  };

  return (
    <div className="automation-editor">
      <div className="automation-editor-group">
        <label className="automation-editor-label">Description</label>
        <Input
          value={editDesc}
          onChange={(e) => onDescChange(e.target.value)}
          placeholder="Workflow description..."
        />
      </div>

      {/* Tab bar */}
      {isToml && (
        <div className="wf-editor-tabs">
          <button
            className={`wf-editor-tab ${tab === 'structured' ? 'active' : ''}`}
            onClick={() => setTab('structured')}
          >
            Structured
          </button>
          <button
            className={`wf-editor-tab ${tab === 'source' ? 'active' : ''}`}
            onClick={() => setTab('source')}
          >
            Source
          </button>
        </div>
      )}

      {/* Structured tab */}
      {tab === 'structured' && isToml && (
        <div className="wf-task-cards">
          {parseError && (
            <div className="automation-validation invalid">
              <AlertCircle size={14} /> {parseError}
            </div>
          )}
          {tasks.map((task, idx) => (
            <div key={idx} className="wf-task-card">
              <div className="wf-task-card-header">
                <span className="wf-task-card-index">Task {idx + 1}</span>
                <button
                  className="wf-task-card-remove"
                  onClick={() => handleRemoveTask(idx)}
                  title="Remove task"
                >
                  <X size={14} />
                </button>
              </div>
              <div className="wf-task-card-grid">
                <div className="wf-task-field">
                  <label>ID</label>
                  <Input
                    value={task.id}
                    onChange={(e) => handleTaskChange(idx, 'id', e.target.value)}
                    placeholder="step1"
                  />
                </div>
                <div className="wf-task-field">
                  <label>Name</label>
                  <Input
                    value={task.name}
                    onChange={(e) => handleTaskChange(idx, 'name', e.target.value)}
                    placeholder="Step Name"
                  />
                </div>
                <div className="wf-task-field">
                  <label>Type</label>
                  <Select
                    value={task.type}
                    onValueChange={(val) => handleTaskChange(idx, 'type', val)}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="tool_execution">tool_execution</SelectItem>
                      <SelectItem value="agent_task">agent_task</SelectItem>
                      <SelectItem value="conditional">conditional</SelectItem>
                      <SelectItem value="parallel_group">parallel_group</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="wf-task-field">
                  <label>Tool / Agent</label>
                  <Input
                    value={task.tool_name}
                    onChange={(e) => handleTaskChange(idx, 'tool_name', e.target.value)}
                    placeholder="ShellExec, WebFetch..."
                  />
                </div>
                <div className="wf-task-field">
                  <label>Depends On</label>
                  <Input
                    value={task.depends_on}
                    onChange={(e) => handleTaskChange(idx, 'depends_on', e.target.value)}
                    placeholder='["step1"]'
                  />
                </div>
                <div className="wf-task-field">
                  <label>Retry</label>
                  <Input
                    value={task.retry}
                    onChange={(e) => handleTaskChange(idx, 'retry', e.target.value)}
                    placeholder="0"
                  />
                </div>
                <div className="wf-task-field">
                  <label>Failure Strategy</label>
                  <Select
                    value={task.failure_strategy || 'default'}
                    onValueChange={(val) => handleTaskChange(idx, 'failure_strategy', val === 'default' ? '' : val)}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="default">default</SelectItem>
                      <SelectItem value="stop">stop</SelectItem>
                      <SelectItem value="continue">continue</SelectItem>
                      <SelectItem value="retry">retry</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>
              {/* Parameters */}
              <div className="wf-task-params">
                <label>Parameters (TOML key = value)</label>
                <Textarea
                  value={task.parameters}
                  onChange={(e) => handleTaskChange(idx, 'parameters', e.target.value)}
                  placeholder={'command = "df -h"\nurl = "https://..."'}
                  rows={3}
                  spellCheck={false}
                />
              </div>
            </div>
          ))}
          <Button variant="outline" size="sm" className="wf-add-task self-start" onClick={handleAddTask}>
            + Add Task
          </Button>
        </div>
      )}

      {/* Source tab (always available for DSL, fallback for TOML) */}
      {(tab === 'source' || !isToml) && (
        <div className="automation-editor-group">
          <label className="automation-editor-label">Definition ({format})</label>
          <Textarea
            value={editDef}
            onChange={(e) => onDefChange(e.target.value)}
            spellCheck={false}
            rows={12}
          />
        </div>
      )}

      {validation && (
        <div className={`automation-validation ${validation.valid ? 'valid' : 'invalid'}`}>
          {validation.valid ? (
            <><CheckCircle size={14} /> Valid</>
          ) : (
            <><AlertCircle size={14} /> {validation.errors.join('; ')}</>
          )}
        </div>
      )}
      <div className="automation-editor-actions">
        <Button variant="outline" onClick={onValidate}>
          Validate
        </Button>
        <Button variant="primary" onClick={onSave} disabled={saving}>
          <Save size={14} /> {saving ? 'Saving...' : 'Save'}
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          <X size={14} /> Cancel
        </Button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ScheduleDetail
// ---------------------------------------------------------------------------

function ScheduleDetail({
  id,
  workflows,
  getSchedule,
  updateSchedule,
  deleteSchedule,
  pauseSchedule,
  resumeSchedule,
  getExecutionHistory,
  triggerScheduleNow,
}: {
  id: string;
  workflows: WorkflowInfo[];
  getSchedule: (id: string) => Promise<ScheduleInfo | null>;
  updateSchedule: (id: string, request: {
    name?: string;
    trigger?: unknown;
    workflow_id?: string;
    description?: string;
  }) => Promise<ScheduleInfo | null>;
  deleteSchedule: (id: string) => Promise<boolean>;
  pauseSchedule: (id: string) => Promise<boolean>;
  resumeSchedule: (id: string) => Promise<boolean>;
  getExecutionHistory: (scheduleId: string) => Promise<ExecutionRecord[]>;
  triggerScheduleNow: (scheduleId: string) => Promise<ExecutionRecord | null>;
}) {
  const [sc, setSc] = useState<ScheduleInfo | null>(null);
  const [execHistory, setExecHistory] = useState<ExecutionRecord[]>([]);
  const [triggering, setTriggering] = useState(false);

  // Edit trigger state
  const [editingTrigger, setEditingTrigger] = useState(false);
  const [editTriggerType, setEditTriggerType] = useState('');
  const [editTriggerValue, setEditTriggerValue] = useState('');
  const [savingTrigger, setSavingTrigger] = useState(false);

  const load = useCallback(async () => {
    const s = await getSchedule(id);
    setSc(s);
    const history = await getExecutionHistory(id);
    setExecHistory(history);
  }, [id, getSchedule, getExecutionHistory]);

  // eslint-disable-next-line react-hooks/set-state-in-effect
  useEffect(() => { load(); setEditingTrigger(false); }, [load]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void load();
    }, 5000);
    return () => window.clearInterval(timer);
  }, [load]);

  const handleTriggerNow = async () => {
    setTriggering(true);
    await triggerScheduleNow(id);
    setTriggering(false);
    const history = await getExecutionHistory(id);
    setExecHistory(history);
  };

  const handleStartEditTrigger = () => {
    if (!sc) return;
    setEditTriggerType(sc.trigger_type);
    // Parse the display value back to editable value
    if (sc.trigger_type === 'cron') {
      // trigger_value is "expr (tz)" - extract expression
      const match = sc.trigger_value.match(/^(.+?)\s*\(/);
      setEditTriggerValue(match ? match[1].trim() : sc.trigger_value);
    } else if (sc.trigger_type === 'interval') {
      // trigger_value is "300s" - extract number
      setEditTriggerValue(sc.trigger_value.replace(/s$/, ''));
    } else {
      setEditTriggerValue(sc.trigger_value);
    }
    setEditingTrigger(true);
  };

  const handleSaveTrigger = async () => {
    if (!sc) return;
    setSavingTrigger(true);
    let trigger: unknown;
    if (editTriggerType === 'cron') {
      trigger = { type: 'cron', expression: editTriggerValue, timezone: 'UTC' };
    } else if (editTriggerType === 'interval') {
      trigger = { type: 'interval', interval_secs: parseInt(editTriggerValue, 10) || 3600 };
    } else {
      trigger = {
        type: 'one_time',
        at: new Date(Date.now() + (parseInt(editTriggerValue, 10) || 60) * 1000).toISOString(),
      };
    }
    await updateSchedule(id, { trigger });
    setSavingTrigger(false);
    setEditingTrigger(false);
    load();
  };

  if (!sc) {
    return <div className="automation-panel"><div className="automation-loading">Loading schedule...</div></div>;
  }

  const linkedWf = workflows.find((w) => w.id === sc.workflow_id);

  return (
    <div className="automation-panel">
      <div className="automation-detail">
        <div className="automation-detail-header">
          <div className="automation-detail-info">
            <div className="automation-detail-title-row">
              <Clock size={20} className="automation-detail-icon" />
              <h2 className="automation-detail-name">{sc.name}</h2>
            </div>
            {sc.description && <p className="automation-detail-desc">{sc.description}</p>}
            <div className="automation-detail-badges">
              <span className={`automation-badge automation-badge--${sc.trigger_type}`}>
                {sc.trigger_type}
              </span>
              <span className={`automation-badge ${sc.enabled ? 'automation-badge--active' : 'automation-badge--paused'}`}>
                {sc.enabled ? 'Active' : 'Paused'}
              </span>
            </div>
          </div>
          <div className="automation-detail-actions">
            <Button
              variant="outline"
              size="sm"
              onClick={handleTriggerNow}
              disabled={triggering}
            >
              <RotateCw size={14} className={triggering ? 'spin-animation' : ''} />
              {triggering ? 'Triggering...' : 'Trigger Now'}
            </Button>
            {sc.enabled ? (
              <Button variant="ghost" size="sm" onClick={() => pauseSchedule(id).then(() => load())}>
                <Pause size={14} /> Pause
              </Button>
            ) : (
              <Button variant="outline" size="sm" onClick={() => resumeSchedule(id).then(() => load())}>
                <Play size={14} /> Resume
              </Button>
            )}
            <Button variant="danger" size="sm" onClick={async () => {
              await deleteSchedule(id);
              setSc(null);
            }}>
              <Trash2 size={14} /> Delete
            </Button>
          </div>
        </div>

        <div className="automation-section">
          <div className="automation-section-title">Configuration</div>
          <div className="automation-meta-grid">
            <div className="automation-meta-item">
              <span className="automation-meta-label">Trigger Type</span>
              <span className="automation-meta-value">{sc.trigger_type}</span>
            </div>
            <div className="automation-meta-item">
              <span className="automation-meta-label">Trigger Value</span>
              <span className="automation-meta-value automation-meta-mono">{sc.trigger_value}</span>
            </div>
            <div className="automation-meta-item">
              <span className="automation-meta-label">Linked Workflow</span>
              <span className="automation-meta-value">{linkedWf?.name ?? sc.workflow_id}</span>
            </div>
            <div className="automation-meta-item">
              <span className="automation-meta-label">Created</span>
              <span className="automation-meta-value">{new Date(sc.created_at).toLocaleString()}</span>
            </div>
            {sc.last_fire && (
              <div className="automation-meta-item">
                <span className="automation-meta-label">Last Fired</span>
                <span className="automation-meta-value">{new Date(sc.last_fire).toLocaleString()}</span>
              </div>
            )}
          </div>

          {/* Edit Trigger */}
          {!editingTrigger ? (
            <Button variant="outline" size="sm" className="mt-3 self-start" onClick={handleStartEditTrigger}>
              <Pencil size={14} /> Edit Trigger
            </Button>
          ) : (
            <div className="schedule-trigger-editor max-w-sm mt-3">
              <div className="automation-editor-group">
                <label className="automation-editor-label">Trigger Type</label>
                <Select
                  value={editTriggerType}
                  onValueChange={setEditTriggerType}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="cron">Cron</SelectItem>
                    <SelectItem value="interval">Interval</SelectItem>
                    <SelectItem value="onetime">One-time</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="automation-editor-group">
                <label className="automation-editor-label">
                  {editTriggerType === 'cron' ? 'Cron Expression' : editTriggerType === 'interval' ? 'Interval (seconds)' : 'Delay (seconds)'}
                </label>
                <Input
                  value={editTriggerValue}
                  onChange={(e) => setEditTriggerValue(e.target.value)}
                  placeholder={editTriggerType === 'cron' ? '0 9 * * 1-5' : '3600'}
                />
              </div>
              <div className="automation-editor-actions mt-2">
                <Button variant="primary" size="sm" onClick={handleSaveTrigger} disabled={savingTrigger}>
                  <Save size={14} /> {savingTrigger ? 'Saving...' : 'Save Trigger'}
                </Button>
                <Button variant="ghost" size="sm" onClick={() => setEditingTrigger(false)}>
                  <X size={14} /> Cancel
                </Button>
              </div>
            </div>
          )}
        </div>

        {/* Execution History */}
        <div className="automation-section">
          <div className="automation-section-title">Execution History</div>
          <ExecutionHistorySection executions={execHistory} />
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ExecutionHistorySection
// ---------------------------------------------------------------------------

function ExecutionHistorySection({ executions }: { executions: ExecutionRecord[] }) {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [copiedField, setCopiedField] = useState<string | null>(null);

  const handleCopy = async (text: string, fieldId: string) => {
    try {
      if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
        await navigator.clipboard.writeText(text);
      } else {
        // Fallback for insecure contexts.
        const ta = document.createElement('textarea');
        ta.value = text;
        ta.style.position = 'fixed';
        ta.style.left = '-9999px';
        document.body.appendChild(ta);
        ta.select();
        document.execCommand('copy');
        document.body.removeChild(ta);
      }
      setCopiedField(fieldId);
      setTimeout(() => setCopiedField(null), 1500);
    } catch (e) {
      console.error('Copy failed:', e);
    }
  };

  if (executions.length === 0) {
    return (
      <div className="exec-history-empty">
        No executions yet. Click "Trigger Now" to run this schedule.
      </div>
    );
  }

  return (
    <div className="exec-history-list">
      {executions.map((exec) => {
        const isExpanded = expandedId === exec.execution_id;
        const statusClass = `exec-status--${exec.status}`;
        const triggeredDate = new Date(exec.triggered_at);

        return (
          <div key={exec.execution_id} className="exec-history-entry">
            <button
              className="exec-history-header"
              onClick={() => setExpandedId(isExpanded ? null : exec.execution_id)}
            >
              <span className="exec-history-chevron">
                {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              </span>
              <span className={`exec-status-badge ${statusClass}`}>
                {exec.status}
              </span>
              <span className="exec-history-time">
                {triggeredDate.toLocaleString()}
              </span>
              {exec.duration_ms !== null && (
                <span className="exec-history-duration">
                  {exec.duration_ms < 1000
                    ? `${exec.duration_ms}ms`
                    : `${(exec.duration_ms / 1000).toFixed(1)}s`}
                </span>
              )}
              <span className="exec-history-id">
                {exec.execution_id.length > 20
                  ? `...${exec.execution_id.slice(-12)}`
                  : exec.execution_id}
              </span>
            </button>

            {isExpanded && (
              <div className="exec-history-detail">
                {/* Error message */}
                {exec.error_message && (
                  <div className="exec-detail-error">
                    <span className="exec-detail-label">Error</span>
                    <div className="exec-detail-error-message">{exec.error_message}</div>
                  </div>
                )}

                {/* Request summary */}
                <div className="exec-detail-section">
                  <div className="exec-detail-section-header">
                    <span className="exec-detail-label">Request</span>
                    <button
                      className="exec-copy-btn"
                      onClick={() => handleCopy(
                        JSON.stringify(exec.request_summary, null, 2),
                        `req-${exec.execution_id}`,
                      )}
                    >
                      <Copy size={12} />
                      {copiedField === `req-${exec.execution_id}` ? 'Copied' : 'Copy'}
                    </button>
                  </div>
                  <pre className="exec-detail-json">
                    {JSON.stringify(exec.request_summary, null, 2)}
                  </pre>
                </div>

                {/* Response summary */}
                <div className="exec-detail-section">
                  <div className="exec-detail-section-header">
                    <span className="exec-detail-label">Response</span>
                    <button
                      className="exec-copy-btn"
                      onClick={() => handleCopy(
                        JSON.stringify(exec.response_summary, null, 2),
                        `res-${exec.execution_id}`,
                      )}
                    >
                      <Copy size={12} />
                      {copiedField === `res-${exec.execution_id}` ? 'Copied' : 'Copy'}
                    </button>
                  </div>
                  <pre className="exec-detail-json">
                    {JSON.stringify(exec.response_summary, null, 2)}
                  </pre>
                </div>

                {/* Additional metadata */}
                <div className="exec-detail-meta">
                  {exec.workflow_execution_id && (
                    <span className="exec-detail-meta-item">
                      Workflow: <code>{exec.workflow_execution_id}</code>
                    </span>
                  )}
                  {exec.started_at && (
                    <span className="exec-detail-meta-item">
                      Started: {new Date(exec.started_at).toLocaleTimeString()}
                    </span>
                  )}
                  {exec.completed_at && (
                    <span className="exec-detail-meta-item">
                      Completed: {new Date(exec.completed_at).toLocaleTimeString()}
                    </span>
                  )}
                </div>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// WorkflowCreateForm
// ---------------------------------------------------------------------------

function WorkflowCreateForm({
  onSave,
  onValidate,
  onCancel,
}: {
  onSave: (name: string, definition: string, format: string, description?: string, tags?: string) => Promise<WorkflowInfo | null>;
  onValidate: (definition: string, format: string) => Promise<ValidationResult | null>;
  onCancel: () => void;
}) {
  const [name, setName] = useState('');
  const [definition, setDefinition] = useState('');
  const [format, setFormat] = useState('expression_dsl');
  const [description, setDescription] = useState('');
  const [tags, setTags] = useState('');
  const [validation, setValidation] = useState<ValidationResult | null>(null);
  const [saving, setSaving] = useState(false);

  const handleValidate = async () => {
    const result = await onValidate(definition, format);
    setValidation(result);
  };

  const handleSave = async () => {
    setSaving(true);
    await onSave(name, definition, format, description || undefined, tags || undefined);
    setSaving(false);
    onCancel();
  };

  return (
    <div className="automation-panel">
      <div className="automation-detail">
        <div className="automation-detail-header">
          <div className="automation-detail-info">
            <div className="automation-detail-title-row">
              <GitBranch size={20} className="automation-detail-icon" />
              <h2 className="automation-detail-name">New Workflow</h2>
            </div>
          </div>
        </div>

        <div className="automation-editor">
          <div className="automation-editor-group">
            <label className="automation-editor-label">Name</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-workflow"
            />
          </div>
          <div className="automation-editor-group">
            <label className="automation-editor-label">Format</label>
            <Select
              value={format}
              onValueChange={setFormat}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="expression_dsl">Expression DSL</SelectItem>
                <SelectItem value="toml">TOML</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="automation-editor-group">
            <label className="automation-editor-label">Description</label>
            <Input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What does this workflow do..."
            />
          </div>
          <div className="automation-editor-group">
            <label className="automation-editor-label">Tags (comma-separated)</label>
            <Input
              value={tags}
              onChange={(e) => setTags(e.target.value)}
              placeholder="research, llm"
            />
          </div>
          <div className="automation-editor-group">
            <label className="automation-editor-label">Definition</label>
            <Textarea
              value={definition}
              onChange={(e) => setDefinition(e.target.value)}
              spellCheck={false}
              rows={10}
              placeholder={format === 'expression_dsl'
                ? 'search >> (analyze | score) >> summarize'
                : '[workflow]\nname = "my-flow"\n\n[[tasks]]\nid = "step1"\ntype = "tool"\ntool = "ShellExec"'}
            />
          </div>

          {validation && (
            <div className={`automation-validation ${validation.valid ? 'valid' : 'invalid'}`}>
              {validation.valid ? (
                <><CheckCircle size={14} /> Valid</>
              ) : (
                <><AlertCircle size={14} /> {validation.errors.join('; ')}</>
              )}
            </div>
          )}

          {validation?.valid && validation.dag && (
            <div className="automation-section">
              <div className="automation-section-title">Preview</div>
              <div className="automation-dag-container">
                <DagGraph dag={validation.dag} />
              </div>
            </div>
          )}

          <div className="automation-editor-actions">
            <Button variant="outline" onClick={handleValidate}>
              Validate
            </Button>
            <Button
              variant="primary"
              onClick={handleSave}
              disabled={!name || !definition || saving}
            >
              <Save size={14} /> {saving ? 'Creating...' : 'Create'}
            </Button>
            <Button variant="ghost" onClick={onCancel}>
              <X size={14} /> Cancel
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ScheduleCreateForm
// ---------------------------------------------------------------------------

function ScheduleCreateForm({
  workflows,
  onSave,
  onCancel,
}: {
  workflows: WorkflowInfo[];
  onSave: (request: { name: string; trigger: unknown; workflow_id: string; description?: string }) => Promise<ScheduleInfo | null>;
  onCancel: () => void;
}) {
  const [name, setName] = useState('');
  const [triggerType, setTriggerType] = useState('interval');
  const [triggerValue, setTriggerValue] = useState('3600');
  const [workflowId, setWorkflowId] = useState(workflows[0]?.id ?? '');
  const [description, setDescription] = useState('');
  const [saving, setSaving] = useState(false);

  const handleSave = async () => {
    let trigger: unknown;
    if (triggerType === 'cron') {
      trigger = { type: 'cron', expression: triggerValue, timezone: 'UTC' };
    } else if (triggerType === 'interval') {
      trigger = { type: 'interval', interval_secs: parseInt(triggerValue, 10) || 3600 };
    } else {
      trigger = {
        type: 'one_time',
        at: new Date(Date.now() + (parseInt(triggerValue, 10) || 60) * 1000).toISOString(),
      };
    }

    setSaving(true);
    await onSave({
      name,
      trigger,
      workflow_id: workflowId,
      description: description || undefined,
    });
    setSaving(false);
    onCancel();
  };

  return (
    <div className="automation-panel">
      <div className="automation-detail">
        <div className="automation-detail-header">
          <div className="automation-detail-info">
            <div className="automation-detail-title-row">
              <Clock size={20} className="automation-detail-icon" />
              <h2 className="automation-detail-name">New Schedule</h2>
            </div>
          </div>
        </div>

        <div className="automation-editor">
          <div className="automation-editor-group">
            <label className="automation-editor-label">Name</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="daily-research"
            />
          </div>
          <div className="automation-editor-group">
            <label className="automation-editor-label">Trigger Type</label>
            <Select
              value={triggerType}
              onValueChange={setTriggerType}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="cron">Cron</SelectItem>
                <SelectItem value="interval">Interval</SelectItem>
                <SelectItem value="onetime">One-time</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="automation-editor-group">
            <label className="automation-editor-label">
              {triggerType === 'cron' ? 'Cron Expression' : triggerType === 'interval' ? 'Interval (seconds)' : 'Delay (seconds)'}
            </label>
            <Input
              value={triggerValue}
              onChange={(e) => setTriggerValue(e.target.value)}
              placeholder={triggerType === 'cron' ? '0 9 * * 1-5' : '3600'}
            />
          </div>
          <div className="automation-editor-group">
            <label className="automation-editor-label">Workflow</label>
            <Select
              value={workflowId}
              onValueChange={setWorkflowId}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {workflows.length === 0 && <SelectItem value="__none__" disabled>No workflows available</SelectItem>}
                {workflows.map((w) => (
                  <SelectItem key={w.id} value={w.id}>{w.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="automation-editor-group">
            <label className="automation-editor-label">Description</label>
            <Input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Schedule description..."
            />
          </div>

          <div className="automation-editor-actions">
            <Button
              variant="primary"
              onClick={handleSave}
              disabled={!name || !workflowId || saving}
            >
              <Save size={14} /> {saving ? 'Creating...' : 'Create'}
            </Button>
            <Button variant="ghost" onClick={onCancel}>
              <X size={14} /> Cancel
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
