// ---------------------------------------------------------------------------
// ProviderMigrationPanel -- quick-import LLM provider configs from external
// agent CLIs into providers.toml by clicking a provider logo.
//
// Renders a fixed-order row of source logos (omp, kimi, claude, codex, omo).
// Each tile reflects the detection state reported by the backend:
//   - not detected   -> dimmed, not clickable
//   - unsupported    -> dimmed, not clickable (e.g. codex OAuth)
//   - actionable     -> clickable, opens a selection dialog
//   - migrated       -> green check overlay, locked
// ---------------------------------------------------------------------------

import { useState, useEffect, useCallback } from 'react';
import { Check } from 'lucide-react';
import { transport } from '../../lib';
import type {
  MigrationSourceId,
  MigrationSourceInfo,
  MigrationReport,
} from '../../types';
import { ProviderIconImg } from './ProviderIconPicker';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  Checkbox,
  Button,
} from '../ui';
import './ProviderMigrationPanel.css';

// Fixed display order for the supported migration sources.
const SOURCE_ORDER: MigrationSourceId[] = ['omp', 'kimi', 'claude', 'codex', 'omo'];

// Fallback labels used when a source is not reported by the backend (e.g. the
// config file does not exist, so detection returns no entry).
const FALLBACK_LABEL: Record<MigrationSourceId, string> = {
  omp: 'omp',
  kimi: 'Kimi',
  claude: 'Claude',
  codex: 'Codex',
  omo: 'omo',
};

interface ProviderMigrationPanelProps {
  /** Called after a successful migration so the host can refresh its own state. */
  onMigrated?: (report: MigrationReport) => void;
  /** Optional toast sink (ProvidersTab passes one). */
  setToast?: (t: { message: string; type: 'success' | 'error' } | null) => void;
  /** When false, hide the built-in header (for embedding in a host Dialog). */
  showHeader?: boolean;
}

type TileState = 'not-detected' | 'unsupported' | 'actionable' | 'migrated';

function tileState(source: MigrationSourceInfo | undefined): TileState {
  if (!source || !source.detected) return 'not-detected';
  if (!source.supported) return 'unsupported';
  if (source.migrated) return 'migrated';
  return 'actionable';
}

/**
 * Circular badge showing a tool's initials, used for sources that do not have
 * a lobehub icon (omp / omo). Matches the visual weight of the icon tiles.
 */
function MigrationLogoBadge({ initials }: { initials: string }) {
  return (
    <span className="provider-migration-badge" aria-hidden="true">
      {initials}
    </span>
  );
}

export function ProviderMigrationPanel({ onMigrated, setToast, showHeader = true }: ProviderMigrationPanelProps) {
  const [sources, setSources] = useState<MigrationSourceInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [dialogSource, setDialogSource] = useState<MigrationSourceInfo | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [migrating, setMigrating] = useState(false);
  const [localToast, setLocalToast] = useState<{ message: string; type: 'success' | 'error' } | null>(null);

  const detect = useCallback(async () => {
    try {
      const result = await transport.invoke<MigrationSourceInfo[]>('provider_migration_detect');
      setSources(result ?? []);
    } catch {
      setSources([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    detect();
  }, [detect]);

  const notify = useCallback((message: string, type: 'success' | 'error') => {
    if (setToast) {
      setToast({ message, type });
    } else {
      setLocalToast({ message, type });
      window.setTimeout(() => setLocalToast(null), 4000);
    }
  }, [setToast]);

  const openDialog = useCallback((source: MigrationSourceInfo) => {
    setDialogSource(source);
    setSelectedIds(new Set(source.providers.map((p) => p.id)));
  }, []);

  const closeDialog = useCallback(() => {
    setDialogSource(null);
    setSelectedIds(new Set());
  }, []);

  const toggleCandidate = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const handleConfirm = useCallback(async () => {
    if (!dialogSource || migrating) return;
    const sourceId = dialogSource.id;
    const ids = Array.from(selectedIds);
    setMigrating(true);
    try {
      const report = await transport.invoke<MigrationReport>('provider_migration_run', {
        sourceId,
        selectedIds: ids,
      });
      const count = report?.imported?.length ?? 0;
      notify(`迁移完成：已导入 ${count} 个 provider`, 'success');
      closeDialog();
      await detect();
      onMigrated?.(report);
    } catch (e) {
      notify(`迁移失败：${String(e)}`, 'error');
    } finally {
      setMigrating(false);
    }
  }, [dialogSource, selectedIds, migrating, notify, closeDialog, detect, onMigrated]);

  const sourceMap = new Map<MigrationSourceId, MigrationSourceInfo>();
  for (const s of sources) sourceMap.set(s.id, s);

  return (
    <div className="provider-migration-panel">
      {showHeader && (
        <div className="provider-migration-panel-header">
          <h3 className="provider-migration-panel-title">快速导入 Provider</h3>
          <span className="provider-migration-panel-hint">从其他 agent CLI 导入已配置的 provider</span>
        </div>
      )}

      <div className="provider-migration-logos">
        {SOURCE_ORDER.map((id) => {
          const source = sourceMap.get(id);
          const state = tileState(source);
          const label = source?.label ?? FALLBACK_LABEL[id];
          const clickable = state === 'actionable' && !!source;
          let tooltip: string;
          if (state === 'not-detected') {
            tooltip = `未检测到 ${label} 配置`;
          } else if (state === 'unsupported') {
            tooltip = source?.unsupported_reason ?? `不支持从 ${label} 迁移`;
          } else if (state === 'migrated') {
            tooltip = `已从 ${label} 导入`;
          } else {
            tooltip = `从 ${label} 导入 provider`;
          }

          const tileClasses = [
            'provider-migration-tile',
            state === 'not-detected' || state === 'unsupported'
              ? 'provider-migration-tile--dimmed'
              : state === 'migrated'
                ? 'provider-migration-tile--locked'
                : 'provider-migration-tile--clickable',
          ].join(' ');

          return (
            <button
              key={id}
              type="button"
              className={tileClasses}
              title={tooltip}
              aria-label={`${label} (${tooltip})`}
              data-testid={`migration-tile-${id}`}
              data-state={state}
              disabled={!clickable}
              onClick={clickable && source ? () => openDialog(source) : undefined}
            >
              {source?.icon_id ? (
                <ProviderIconImg iconId={source.icon_id} size={24} />
              ) : (
                <MigrationLogoBadge initials={id} />
              )}
              {state === 'migrated' && (
                <span className="provider-migration-check" aria-label="已迁移">
                  <Check size={12} strokeWidth={3} />
                </span>
              )}
            </button>
          );
        })}
      </div>

      {loading && (
        <div className="provider-migration-panel-hint" style={{ marginTop: 8 }}>
          正在检测可迁移的配置...
        </div>
      )}

      {localToast && (
        <div
          className="provider-migration-panel-hint"
          data-testid="migration-local-toast"
          style={{ marginTop: 8 }}
        >
          {localToast.message}
        </div>
      )}

      <Dialog
        open={dialogSource !== null}
        onOpenChange={(open) => {
          if (!open) closeDialog();
        }}
      >
        {dialogSource && (
          <DialogContent size="md">
            <DialogTitle>迁移 {dialogSource.label} 的 provider</DialogTitle>
            <DialogDescription>
              选择要导入的 provider，确认后将写入 providers.toml。
            </DialogDescription>

            <div className="provider-migration-dialog-body">
              {dialogSource.providers.map((c) => {
              const checked = selectedIds.has(c.id);
              return (
                <div key={c.id} className="provider-migration-candidate" data-testid={`migration-candidate-${c.id}`}>
                  <Checkbox
                    checked={checked}
                    onCheckedChange={() => toggleCandidate(c.id)}
                    aria-label={c.label}
                  />
                  <div className="provider-migration-candidate-main">
                    <span className="provider-migration-candidate-label">{c.label}</span>
                    <span className="provider-migration-candidate-meta">{c.model}</span>
                    <span className="provider-migration-candidate-meta">{c.base_url ?? '—'}</span>
                    <span className="provider-migration-candidate-key">
                      {c.has_api_key ? `key: ${c.api_key_preview}` : '无 API key'}
                    </span>
                  </div>
                </div>
              );
              })}
            </div>

            <div className="flex gap-2 w-full mt-2">
              <Button
                variant="ghost"
                className="flex-1"
                onClick={closeDialog}
                disabled={migrating}
              >
                取消
              </Button>
              <Button
                variant="primary"
                className="flex-1"
                onClick={handleConfirm}
                disabled={migrating || selectedIds.size === 0}
                data-testid="migration-confirm"
              >
                {migrating ? '迁移中...' : '迁移选中'}
              </Button>
            </div>
          </DialogContent>
        )}
      </Dialog>
    </div>
  );
}
