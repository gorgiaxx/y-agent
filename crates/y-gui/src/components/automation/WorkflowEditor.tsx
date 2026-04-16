import { useState, useEffect } from 'react';
import { AlertCircle, CheckCircle, Save, X } from 'lucide-react';
import {
  Button,
  Input,
  Textarea,
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
  Tabs,
  TabsList,
  TabsTrigger,
} from '../ui';
import type { ValidationResult } from './types';
import './AutomationPanel.css';

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

export function WorkflowEditor({
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
        <Tabs value={tab} onValueChange={(v) => setTab(v as 'structured' | 'source')} className="mb-1">
          <TabsList>
            <TabsTrigger value="structured">Structured</TabsTrigger>
            <TabsTrigger value="source">Source</TabsTrigger>
          </TabsList>
        </Tabs>
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
