import { useState, useEffect, useCallback } from 'react';
import { GitBranch, Pencil, Trash2, Play, CheckCircle, AlertCircle, RotateCw } from 'lucide-react';
import { DagGraph } from './DagGraph';
import { ExecutionHistorySection } from './ExecutionHistorySection';
import { WorkflowEditor } from './WorkflowEditor';
import type { WorkflowInfo, DagVisualization, ValidationResult, ExecutionRecord } from './types';
import '../automation/AutomationPanel.css';

export function WorkflowDetail({
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
