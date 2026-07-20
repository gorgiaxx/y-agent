import { useCallback, useEffect, useMemo, useState } from 'react';
import { FolderOpen, RefreshCw, ShieldCheck, Trash2, Undo2 } from 'lucide-react';

import { getWorkspaceTrust } from '../../hooks/useWorkspaces';
import { useSessionInteractions } from '../../hooks/useSessionInteractions';
import { platform, transport } from '../../lib';
import type {
  CapabilityPackInspection,
  InstalledCapabilityPackSummary,
  RuntimeFeatureAvailability,
  SessionInfo,
  WorkspaceInfo,
  WorkspaceTrustDecision,
} from '../../types';
import { PermissionDialog } from '../chat-panel/input-area/PermissionDialog';
import { Button, Checkbox, Input, SettingsGroup, SettingsItem } from '../ui';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/Select';
import { FeatureAvailabilityNotice } from './FeatureAvailabilityNotice';

interface CapabilityPackInspectionPanelProps {
  inspection: CapabilityPackInspection;
  allowReplacements: boolean;
  installing: boolean;
  onAllowReplacementsChange: (allowed: boolean) => void;
  onInstall: () => void;
}

export function CapabilityPackInspectionPanel({
  inspection,
  allowReplacements,
  installing,
  onAllowReplacementsChange,
  onInstall,
}: CapabilityPackInspectionPanelProps) {
  const pack = inspection.validation.pack;
  const replacements = inspection.preview?.changes.filter((change) => change.change === 'replace') ?? [];

  if (!inspection.validation.valid || !pack || !inspection.preview) {
    return (
      <SettingsGroup title="Validation Failed">
        {inspection.validation.issues.map((issue, index) => (
          <div key={`${issue.code}-${index}`} className="text-11px text-[var(--error)]">
            {issue.code}: {issue.message}
          </div>
        ))}
      </SettingsGroup>
    );
  }

  return (
    <SettingsGroup
      title={`${pack.id} ${pack.version}`}
      description={pack.description ?? 'Validated local Capability Pack'}
    >
      <div className="font-mono text-10px text-[var(--text-muted)]">
        {pack.provenance.manifest_path}
      </div>
      {inspection.preview.changes.map((change) => (
        <div
          key={`${change.resource_kind}:${change.resource_id}`}
          className="flex flex-wrap items-center justify-between gap-2 text-11px"
        >
          <span className="font-mono text-[var(--text-primary)]">
            {change.resource_kind}:{change.resource_id}
          </span>
          <span className="text-[var(--text-muted)]">
            {change.change}
            {change.requires_activation ? ' · Requires separate activation' : ''}
          </span>
        </div>
      ))}
      {replacements.length > 0 && (
        <SettingsItem
          title="Approve Replacements"
          description="Existing user-owned resources are snapshotted and restored by rollback."
        >
          <Checkbox
            checked={allowReplacements}
            onCheckedChange={(checked) => onAllowReplacementsChange(checked === true)}
          />
        </SettingsItem>
      )}
      <div className="flex justify-end">
        <Button
          type="button"
          variant="primary"
          disabled={installing || (replacements.length > 0 && !allowReplacements)}
          onClick={onInstall}
        >
          {installing ? 'Installing...' : 'Install declarative resources'}
        </Button>
      </div>
    </SettingsGroup>
  );
}

interface InstalledCapabilityPackCardProps {
  pack: InstalledCapabilityPackSummary;
  selectedWorkspacePath: string | null;
  busy: boolean;
  onActivate: () => void;
  onRevoke: () => void;
  onRollback: () => void;
  onRemove: () => void;
}

export function InstalledCapabilityPackCard({
  pack,
  selectedWorkspacePath,
  busy,
  onActivate,
  onRevoke,
  onRollback,
  onRemove,
}: InstalledCapabilityPackCardProps) {
  const selectedGrant = selectedWorkspacePath
    ? pack.activation_grants.find((grant) => grant.canonical_workspace === selectedWorkspacePath)
    : undefined;
  const hasExecutableResources = pack.executable_resources.length > 0;

  return (
    <SettingsGroup
      title={`${pack.pack_id} ${pack.current_version}`}
      description={`${pack.resources.length} resources · ${pack.installed_versions.length} installed version(s)`}
    >
      <div className="flex flex-wrap gap-2 text-10px text-[var(--text-muted)]">
        {pack.resources.map((resource) => (
          <span key={resource} className="rounded border border-solid border-[var(--border)] px-2 py-1 font-mono">
            {resource}
          </span>
        ))}
      </div>

      {pack.live_resources.length > 0 && (
        <div className="text-11px text-[var(--success)]">
          Live: {pack.live_resources.join(', ')}
        </div>
      )}
      {selectedGrant && pack.live_resources.length === 0 && (
        <div className="text-11px text-[var(--warning)]">Activation approved, not live</div>
      )}

      <div className="flex flex-wrap justify-end gap-2">
        {hasExecutableResources && !selectedGrant && (
          <Button type="button" size="sm" variant="primary" disabled={busy || !selectedWorkspacePath} onClick={onActivate}>
            <ShieldCheck size={12} />
            Request activation
          </Button>
        )}
        {hasExecutableResources && selectedGrant && pack.live_resources.length === 0 && (
          <Button type="button" size="sm" variant="primary" disabled={busy} onClick={onActivate}>
            <RefreshCw size={12} />
            Retry live activation
          </Button>
        )}
        {selectedGrant && (
          <Button type="button" size="sm" variant="outline" disabled={busy} onClick={onRevoke}>
            Revoke approval
          </Button>
        )}
        <Button type="button" size="sm" variant="outline" disabled={busy} onClick={onRollback}>
          <Undo2 size={12} />
          Roll back current
        </Button>
        <Button type="button" size="sm" variant="danger" disabled={busy} onClick={onRemove}>
          <Trash2 size={12} />
          Remove all versions
        </Button>
      </div>
    </SettingsGroup>
  );
}

interface CapabilityPacksTabProps {
  availability: RuntimeFeatureAvailability | null | undefined;
  availabilityError?: string | null;
}

export function CapabilityPacksTab({
  availability,
  availabilityError,
}: CapabilityPacksTabProps) {
  const [packPath, setPackPath] = useState('');
  const [inspection, setInspection] = useState<CapabilityPackInspection | null>(null);
  const [allowReplacements, setAllowReplacements] = useState(false);
  const [installed, setInstalled] = useState<InstalledCapabilityPackSummary[]>([]);
  const [workspaces, setWorkspaces] = useState<WorkspaceInfo[]>([]);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [selectedWorkspaceId, setSelectedWorkspaceId] = useState('');
  const [selectedSessionId, setSelectedSessionId] = useState('');
  const [workspaceTrustState, setWorkspaceTrustState] = useState<{
    sourcePath: string;
    decision: WorkspaceTrustDecision | null;
  }>({ sourcePath: '', decision: null });
  const [loading, setLoading] = useState(false);
  const [busyPackId, setBusyPackId] = useState<string | null>(null);
  const [activationOperationId, setActivationOperationId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const interactions = useSessionInteractions(selectedSessionId || null);
  const available = availability?.available === true;

  const selectedWorkspace = useMemo(
    () => workspaces.find((workspace) => workspace.id === selectedWorkspaceId) ?? null,
    [selectedWorkspaceId, workspaces],
  );
  const workspaceTrust = workspaceTrustState.sourcePath === selectedWorkspace?.path
    ? workspaceTrustState.decision
    : null;
  const selectedWorkspacePath = workspaceTrust?.status === 'trusted'
    ? workspaceTrust.canonical_path
    : null;

  const refreshInstalled = useCallback(async () => {
    if (!available) return;
    const packs = await transport.invoke<InstalledCapabilityPackSummary[]>('capability_pack_list');
    setInstalled(packs);
  }, [available]);

  useEffect(() => {
    if (!available) return;
    let active = true;
    void Promise.all([
      transport.invoke<WorkspaceInfo[]>('workspace_list'),
      transport.invoke<SessionInfo[]>('session_list'),
      transport.invoke<InstalledCapabilityPackSummary[]>('capability_pack_list'),
    ]).then(([workspaceList, sessionList, packList]) => {
      if (!active) return;
      setWorkspaces(workspaceList);
      setSessions(sessionList);
      setInstalled(packList);
      if (workspaceList[0]) setSelectedWorkspaceId(workspaceList[0].id);
      if (sessionList[0]) setSelectedSessionId(sessionList[0].id);
    }).catch((cause: unknown) => {
      if (active) setError(cause instanceof Error ? cause.message : String(cause));
    });
    return () => { active = false; };
  }, [available]);

  useEffect(() => {
    if (!selectedWorkspace) return;
    let active = true;
    void getWorkspaceTrust(selectedWorkspace.path)
      .then((decision) => {
        if (active) setWorkspaceTrustState({ sourcePath: selectedWorkspace.path, decision });
      })
      .catch(() => {
        if (active) setWorkspaceTrustState({ sourcePath: selectedWorkspace.path, decision: null });
      });
    return () => { active = false; };
  }, [selectedWorkspace]);

  const choosePackFolder = async () => {
    const selected = await platform.openFileDialog({ directory: true });
    if (selected?.[0]) setPackPath(selected[0]);
  };

  const inspectPack = async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await transport.invoke<CapabilityPackInspection>('capability_pack_inspect', { path: packPath.trim() });
      setInspection(next);
      setAllowReplacements(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  };

  const installPack = async () => {
    setLoading(true);
    setError(null);
    try {
      await transport.invoke('capability_pack_install', {
        path: packPath.trim(),
        allowReplacements,
      });
      await refreshInstalled();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  };

  const runPackAction = async (packId: string, action: () => Promise<unknown>) => {
    setBusyPackId(packId);
    setError(null);
    try {
      await action();
      await refreshInstalled();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusyPackId(null);
      setActivationOperationId(null);
    }
  };

  const activatePack = (pack: InstalledCapabilityPackSummary) => {
    if (!selectedWorkspacePath || !selectedSessionId) return;
    const grant = pack.activation_grants.find(
      (candidate) => candidate.canonical_workspace === selectedWorkspacePath,
    );
    if (grant) {
      void runPackAction(pack.pack_id, () => transport.invoke('capability_pack_activate_granted', {
        packId: pack.pack_id,
        workspacePath: selectedWorkspacePath,
      }));
      return;
    }
    const operationId = globalThis.crypto?.randomUUID?.() ?? `capability-pack-${Date.now()}`;
    setActivationOperationId(operationId);
    void runPackAction(pack.pack_id, () => transport.invoke('capability_pack_activate', {
      packId: pack.pack_id,
      workspacePath: selectedWorkspacePath,
      sessionId: selectedSessionId,
      operationId,
    }));
  };

  return (
    <div className="settings-form-wrap">
      <FeatureAvailabilityNotice
        featureName="Capability Packs"
        availability={availability}
        error={availabilityError}
        plural
      />

      {interactions.permissionData && (
        <PermissionDialog
          requestId={interactions.permissionData.requestId}
          toolName={interactions.permissionData.toolName}
          actionDescription={interactions.permissionData.actionDescription}
          reason={interactions.permissionData.reason}
          contentPreview={interactions.permissionData.contentPreview}
          onApprove={interactions.handlePermissionApprove}
          onDeny={interactions.handlePermissionDeny}
          onAllowAllForSession={interactions.handlePermissionAllowAllForSession}
          onApproveAlways={interactions.handlePermissionApproveAlways}
        />
      )}

      {activationOperationId && (
        <div className="flex items-center justify-between gap-2 rounded-md border border-solid border-[var(--warning)] p-3 text-11px">
          <span>Waiting for explicit activation approval or owner startup.</span>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => transport.invoke('chat_cancel', { runId: activationOperationId })}
          >
            Cancel
          </Button>
        </div>
      )}

      {error && <div role="alert" className="text-11px text-[var(--error)]">{error}</div>}

      <fieldset disabled={!available} className="contents">
        <SettingsGroup
          title="Inspect Local Pack"
          description="Validation and preview are side-effect free. Installation never activates MCP, hooks, or LSP declarations."
        >
          <SettingsItem title="Pack Directory" wide>
            <div className="flex w-full gap-2">
              <Input
                value={packPath}
                onChange={(event) => setPackPath(event.target.value)}
                placeholder="/path/to/capability-pack"
                className="flex-1"
              />
              {platform.capabilities.nativeFilePaths && (
                <Button type="button" variant="outline" onClick={() => { void choosePackFolder(); }}>
                  <FolderOpen size={13} />
                  Choose
                </Button>
              )}
              <Button type="button" variant="primary" disabled={loading || !packPath.trim()} onClick={() => { void inspectPack(); }}>
                {loading ? 'Inspecting...' : 'Inspect'}
              </Button>
            </div>
          </SettingsItem>
        </SettingsGroup>

        {inspection && (
          <CapabilityPackInspectionPanel
            inspection={inspection}
            allowReplacements={allowReplacements}
            installing={loading}
            onAllowReplacementsChange={setAllowReplacements}
            onInstall={() => { void installPack(); }}
          />
        )}

        <SettingsGroup
          title="Activation Context"
          description="Executable declarations require a trusted workspace and explicit HITL approval."
        >
          <SettingsItem title="Workspace">
            <Select value={selectedWorkspaceId} onValueChange={setSelectedWorkspaceId}>
              <SelectTrigger className="w-[260px]"><SelectValue placeholder="Select workspace" /></SelectTrigger>
              <SelectContent>
                {workspaces.map((workspace) => (
                  <SelectItem key={workspace.id} value={workspace.id}>{workspace.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </SettingsItem>
          <SettingsItem title="Workspace Trust">
            <span className="text-11px text-[var(--text-secondary)]">
              {workspaceTrust?.status ?? 'unknown'}
            </span>
          </SettingsItem>
          <SettingsItem title="Approval Session">
            <Select value={selectedSessionId} onValueChange={setSelectedSessionId}>
              <SelectTrigger className="w-[260px]"><SelectValue placeholder="Select session" /></SelectTrigger>
              <SelectContent>
                {sessions.map((session) => (
                  <SelectItem key={session.id} value={session.id}>{session.title ?? session.id}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </SettingsItem>
        </SettingsGroup>

        <div className="flex items-center justify-between">
          <h3 className="m-0 text-13px font-600 text-[var(--text-primary)]">Installed Packs</h3>
          <Button type="button" size="sm" variant="outline" onClick={() => { void refreshInstalled(); }}>
            <RefreshCw size={12} />
            Refresh
          </Button>
        </div>
        {installed.length === 0 ? (
          <div className="settings-empty">No Capability Packs installed.</div>
        ) : installed.map((pack) => (
          <InstalledCapabilityPackCard
            key={pack.pack_id}
            pack={pack}
            selectedWorkspacePath={selectedWorkspacePath}
            busy={busyPackId === pack.pack_id}
            onActivate={() => activatePack(pack)}
            onRevoke={() => {
              if (!selectedWorkspacePath) return;
              void runPackAction(pack.pack_id, () => transport.invoke('capability_pack_revoke', {
                packId: pack.pack_id,
                workspacePath: selectedWorkspacePath,
              }));
            }}
            onRollback={() => {
              if (!window.confirm(`Roll back ${pack.pack_id} ${pack.current_version}?`)) return;
              void runPackAction(pack.pack_id, () => transport.invoke('capability_pack_rollback', { packId: pack.pack_id }));
            }}
            onRemove={() => {
              if (!window.confirm(`Remove every installed version of ${pack.pack_id}?`)) return;
              void runPackAction(pack.pack_id, () => transport.invoke('capability_pack_remove', { packId: pack.pack_id }));
            }}
          />
        ))}
      </fieldset>
    </div>
  );
}
