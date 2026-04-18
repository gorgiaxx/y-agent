import { useState, useEffect, useCallback } from 'react';
import { Bot, Save, RotateCcw, RefreshCw, Pencil, X } from 'lucide-react';
import type { AgentDetail } from '../../hooks/useAgents';
import { Button, Badge, Textarea } from '../ui';
import './AgentsPanel.css';

interface AgentsPanelProps {
  agentId: string | null;
  onGetDetail: (id: string) => Promise<AgentDetail | null>;
  onSave: (id: string, tomlContent: string) => Promise<boolean>;
  onReset: (id: string) => Promise<boolean>;
  onReload: () => Promise<boolean>;
}

/** Convert an AgentDetail back to TOML string for editing. */
function detailToToml(d: AgentDetail): string {
  const lines: string[] = [];
  lines.push(`id = ${JSON.stringify(d.id)}`);
  lines.push(`name = ${JSON.stringify(d.name)}`);
  lines.push(`description = ${JSON.stringify(d.description)}`);
  lines.push(`mode = ${JSON.stringify(d.mode)}`);
  lines.push(`trust_tier = "user_defined"`);
  lines.push(`capabilities = [${d.capabilities.map(c => JSON.stringify(c)).join(', ')}]`);
  lines.push(`allowed_tools = [${d.allowed_tools.map(t => JSON.stringify(t)).join(', ')}]`);
  lines.push(`system_prompt = ${JSON.stringify(d.system_prompt)}`);
  lines.push(`skills = [${d.skills.map(s => JSON.stringify(s)).join(', ')}]`);
  lines.push(`preferred_models = [${d.preferred_models.map(m => JSON.stringify(m)).join(', ')}]`);
  lines.push(`fallback_models = [${d.fallback_models.map(m => JSON.stringify(m)).join(', ')}]`);
  lines.push(`provider_tags = [${d.provider_tags.map(t => JSON.stringify(t)).join(', ')}]`);
  if (d.temperature !== null) lines.push(`temperature = ${d.temperature}`);
  if (d.top_p !== null) lines.push(`top_p = ${d.top_p}`);
  lines.push(`max_iterations = ${d.max_iterations}`);
  lines.push(`max_tool_calls = ${d.max_tool_calls}`);
  lines.push(`timeout_secs = ${d.timeout_secs}`);
  lines.push(`context_sharing = ${JSON.stringify(d.context_sharing)}`);
  lines.push(`max_context_tokens = ${d.max_context_tokens}`);
  if (d.max_completion_tokens !== null) lines.push(`max_completion_tokens = ${d.max_completion_tokens}`);
  return lines.join('\n') + '\n';
}

export function AgentsPanel({ agentId, onGetDetail, onSave, onReset, onReload }: AgentsPanelProps) {
  const [detail, setDetail] = useState<AgentDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState(false);
  const [tomlContent, setTomlContent] = useState('');
  const [saving, setSaving] = useState(false);
  const [reloading, setReloading] = useState(false);

  const loadDetail = useCallback(async (id: string) => {
    setLoading(true);
    const d = await onGetDetail(id);
    setDetail(d);
    setLoading(false);
    setEditing(false);
  }, [onGetDetail]);

  useEffect(() => {
    if (!agentId) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setDetail(null);
      setEditing(false);
      return;
    }
    loadDetail(agentId);
  }, [agentId, loadDetail]);

  const handleEdit = () => {
    if (!detail) return;
    setTomlContent(detailToToml(detail));
    setEditing(true);
  };

  const handleCancelEdit = () => {
    setEditing(false);
  };

  const handleSave = async () => {
    if (!agentId) return;
    setSaving(true);
    const ok = await onSave(agentId, tomlContent);
    setSaving(false);
    if (ok) {
      await loadDetail(agentId);
    }
  };

  const handleReset = async () => {
    if (!agentId) return;
    const ok = await onReset(agentId);
    if (ok) {
      await loadDetail(agentId);
    }
  };

  const handleReload = async () => {
    setReloading(true);
    await onReload();
    if (agentId) await loadDetail(agentId);
    setReloading(false);
  };

  // --- Empty state ---
  if (!agentId) {
    return (
      <div className="agents-panel">
        <div className="agents-empty">
          <Bot size={40} className="agents-empty-icon" />
          <p className="agents-empty-title">Select an agent</p>
          <p className="agents-empty-desc">
            Choose an agent from the sidebar to view and edit its configuration.
          </p>
        </div>
      </div>
    );
  }

  // --- Loading state ---
  if (loading || !detail) {
    return (
      <div className="agents-panel">
        <div className="agents-loading">Loading agent…</div>
      </div>
    );
  }

  // --- EDIT mode ---
  if (editing) {
    return (
      <div className="agents-panel">
        <div className="agent-detail">
          <div className="agent-detail-header">
            <div className="agent-detail-info">
              <div className="agent-detail-title-row">
                <Bot size={20} className="agent-detail-icon" />
                <h2 className="agent-detail-name">Editing: {detail.name}</h2>
              </div>
            </div>
            <div className="agent-detail-header-actions">
              <Button
                variant="primary"
                size="sm"
                onClick={handleSave}
                disabled={saving}
                title="Save"
              >
                <Save size={14} />
                {saving ? 'Saving…' : 'Save'}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={handleCancelEdit}
                title="Cancel"
              >
                <X size={14} />
                Cancel
              </Button>
            </div>
          </div>
          <Textarea
            variant="mono"
            className="flex-1 mt-3 min-h-[200px] resize-none bg-[var(--surface-code)]"
            value={tomlContent}
            onChange={(e) => setTomlContent(e.target.value)}
            spellCheck={false}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === 's') {
                e.preventDefault();
                handleSave();
              }
            }}
          />
        </div>
      </div>
    );
  }

  // --- VIEW mode ---
  return (
    <div className="agents-panel">
      <div className="agent-detail">
        {/* Header */}
        <div className="agent-detail-header">
          <div className="agent-detail-info">
            <div className="agent-detail-title-row">
              <Bot size={20} className="agent-detail-icon" />
              <h2 className="agent-detail-name">{detail.name}</h2>
            </div>
            <p className="agent-detail-desc">{detail.description}</p>
            <div className="agent-detail-badges">
              <Badge variant="accent" size="md">{detail.mode}</Badge>
              <Badge variant="accent" size="md">
                {detail.trust_tier}
              </Badge>
              {detail.is_overridden && (
                <Badge variant="warning" size="md">Overridden</Badge>
              )}
            </div>
          </div>
          <div className="agent-detail-header-actions">
            <Button variant="ghost" size="sm" onClick={handleEdit} title="Edit TOML">
              <Pencil size={14} />
              Edit
            </Button>
            {detail.is_overridden && (
              <Button variant="warning" size="sm" onClick={handleReset} title="Reset to Default">
                <RotateCcw size={14} />
                Reset
              </Button>
            )}
            <Button
              variant="ghost"
              size="sm"
              onClick={handleReload}
              disabled={reloading}
              title="Reload from Disk"
            >
              <RefreshCw size={14} className={reloading ? 'agent-spin' : ''} />
              {reloading ? 'Reloading…' : 'Reload'}
            </Button>
          </div>
        </div>

        {/* Configuration */}
        <div className="agent-detail-section">
          <div className="agent-detail-section-title">Configuration</div>
          <div className="agent-detail-field">
            <span className="agent-detail-field-label">Mode</span>
            <span className="agent-detail-field-value">{detail.mode}</span>
          </div>
          <div className="agent-detail-field">
            <span className="agent-detail-field-label">Context Sharing</span>
            <span className="agent-detail-field-value">{detail.context_sharing}</span>
          </div>
          {detail.temperature !== null && (
            <div className="agent-detail-field">
              <span className="agent-detail-field-label">Temperature</span>
              <span className="agent-detail-field-value">{detail.temperature}</span>
            </div>
          )}
          {detail.top_p !== null && (
            <div className="agent-detail-field">
              <span className="agent-detail-field-label">Top P</span>
              <span className="agent-detail-field-value">{detail.top_p}</span>
            </div>
          )}
        </div>

        {/* Limits */}
        <div className="agent-detail-section">
          <div className="agent-detail-section-title">Limits</div>
          <div className="agent-detail-field">
            <span className="agent-detail-field-label">Max Iterations</span>
            <span className="agent-detail-field-value">{detail.max_iterations}</span>
          </div>
          <div className="agent-detail-field">
            <span className="agent-detail-field-label">Max Tool Calls</span>
            <span className="agent-detail-field-value">{detail.max_tool_calls}</span>
          </div>
          <div className="agent-detail-field">
            <span className="agent-detail-field-label">Timeout</span>
            <span className="agent-detail-field-value">{detail.timeout_secs}s</span>
          </div>
          <div className="agent-detail-field">
            <span className="agent-detail-field-label">Max Context Tokens</span>
            <span className="agent-detail-field-value">{detail.max_context_tokens.toLocaleString()}</span>
          </div>
          {detail.max_completion_tokens !== null && (
            <div className="agent-detail-field">
              <span className="agent-detail-field-label">Max Completion Tokens</span>
              <span className="agent-detail-field-value">{detail.max_completion_tokens.toLocaleString()}</span>
            </div>
          )}
        </div>

        {/* Allowed Tools */}
        {detail.allowed_tools.length > 0 && (
          <div className="agent-detail-section">
            <div className="agent-detail-section-title">Allowed Tools</div>
            <div className="agent-detail-tags">
              {detail.allowed_tools.map((tool) => (
                <Badge key={tool} variant="info">{tool}</Badge>
              ))}
            </div>
          </div>
        )}

        {/* Capabilities */}
        {detail.capabilities.length > 0 && (
          <div className="agent-detail-section">
            <div className="agent-detail-section-title">Capabilities</div>
            <div className="agent-detail-tags">
              {detail.capabilities.map((cap) => (
                <Badge key={cap} variant="outline">{cap}</Badge>
              ))}
            </div>
          </div>
        )}

        {/* Model Preferences */}
        {(detail.preferred_models.length > 0 || detail.fallback_models.length > 0 || detail.provider_tags.length > 0) && (
          <div className="agent-detail-section">
            <div className="agent-detail-section-title">Model Preferences</div>
            {detail.preferred_models.length > 0 && (
              <div className="agent-detail-field">
                <span className="agent-detail-field-label">Preferred</span>
                <span className="agent-detail-field-value">{detail.preferred_models.join(', ')}</span>
              </div>
            )}
            {detail.fallback_models.length > 0 && (
              <div className="agent-detail-field">
                <span className="agent-detail-field-label">Fallback</span>
                <span className="agent-detail-field-value">{detail.fallback_models.join(', ')}</span>
              </div>
            )}
            {detail.provider_tags.length > 0 && (
              <div className="agent-detail-field">
                <span className="agent-detail-field-label">Provider Tags</span>
                <span className="agent-detail-field-value">{detail.provider_tags.join(', ')}</span>
              </div>
            )}
          </div>
        )}

        {/* Skills */}
        {detail.skills.length > 0 && (
          <div className="agent-detail-section">
            <div className="agent-detail-section-title">Skills</div>
            <div className="agent-detail-tags">
              {detail.skills.map((skill) => (
                <Badge key={skill} variant="outline">{skill}</Badge>
              ))}
            </div>
          </div>
        )}

        {/* System Prompt */}
        <div className="agent-detail-section">
          <div className="agent-detail-section-title">System Prompt</div>
          <div className="agent-detail-prompt">{detail.system_prompt}</div>
        </div>
      </div>
    </div>
  );
}
