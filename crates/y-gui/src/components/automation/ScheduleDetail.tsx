import { useState, useEffect, useCallback } from 'react';
import { Clock, Pencil, Trash2, Save, X, Play, Pause, RotateCw } from 'lucide-react';
import { Button, Input, Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui';
import { ExecutionHistorySection } from './ExecutionHistorySection';
import type { WorkflowInfo, ScheduleInfo, ExecutionRecord } from './types';
import './AutomationPanel.css';

export function ScheduleDetail({
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
