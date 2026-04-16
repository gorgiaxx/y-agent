import { useState } from 'react';
import { GitBranch, Clock, Save, X, CheckCircle, AlertCircle } from 'lucide-react';
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
import type { WorkflowInfo, ScheduleInfo, ValidationResult } from './types';
import './AutomationPanel.css';

// ---------------------------------------------------------------------------
// WorkflowCreateForm
// ---------------------------------------------------------------------------

export function WorkflowCreateForm({
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

export function ScheduleCreateForm({
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
