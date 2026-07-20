import { useState } from 'react';
import { Plus, X } from 'lucide-react';

import type { RuntimeFeatureAvailability } from '../../types';
import { LSP_SCHEMA } from '../../utils/settingsSchemas';
import { Button, Input, SettingsGroup, SettingsItem, SubListLayout, Switch } from '../ui';
import { FeatureAvailabilityNotice } from './FeatureAvailabilityNotice';
import type { LspFormData, LspServerFormData } from './settingsTypes';
import { emptyLspServer, jsonToLsp } from './settingsTypes';
import { SettingsTabShell } from './SettingsTabShell';
import { TagChipInput } from './TagChipInput';
import { useSettingsTab } from './useSettingsTab';

interface LspServerFieldsProps {
  server: LspServerFormData;
  onChange: (server: LspServerFormData) => void;
}

function LspServerFields({ server, onChange }: LspServerFieldsProps) {
  const update = (patch: Partial<LspServerFormData>) => onChange({ ...server, ...patch });

  return (
    <div className="settings-form-wrap">
      <SettingsGroup title="Server Identity">
        <SettingsItem title="Server ID" wide>
          <Input value={server.id} onChange={(event) => update({ id: event.target.value })} />
        </SettingsItem>
        <SettingsItem title="Command" description="Trusted local executable; y-agent never installs it." wide>
          <Input value={server.command} onChange={(event) => update({ command: event.target.value })} />
        </SettingsItem>
        <SettingsItem title="Arguments" wide>
          <TagChipInput tags={server.args} onChange={(args) => update({ args })} />
        </SettingsItem>
        <SettingsItem title="Language ID" wide>
          <Input value={server.language_id} onChange={(event) => update({ language_id: event.target.value })} />
        </SettingsItem>
      </SettingsGroup>

      <SettingsGroup title="Project Matching">
        <SettingsItem title="File Extensions" description="Extensions are matched longest-first." wide>
          <TagChipInput tags={server.extensions} onChange={(extensions) => update({ extensions })} />
        </SettingsItem>
        <SettingsItem title="Root Markers" description="Project root search never escapes the trusted workspace." wide>
          <TagChipInput tags={server.root_markers} onChange={(root_markers) => update({ root_markers })} />
        </SettingsItem>
        {server.initialization_options !== null && (
          <SettingsItem
            title="Initialization Options"
            description="Existing options are preserved. Use RAW mode to edit the arbitrary TOML value."
          >
            <span className="text-11px text-[var(--text-muted)]">Configured</span>
          </SettingsItem>
        )}
      </SettingsGroup>
    </div>
  );
}

interface LspSettingsFieldsProps {
  form: LspFormData;
  availability: RuntimeFeatureAvailability | null | undefined;
  availabilityError?: string | null;
  onUpdate: (patch: Partial<LspFormData>) => void;
}

export function LspSettingsFields({
  form,
  availability,
  availabilityError,
  onUpdate,
}: LspSettingsFieldsProps) {
  const [activeServerIndex, setActiveServerIndex] = useState(0);
  const unavailable = !availability?.available;
  const selectedIndex = Math.min(activeServerIndex, Math.max(0, form.servers.length - 1));
  const selectedServer = form.servers[selectedIndex];

  const changeServer = (server: LspServerFormData) => {
    onUpdate({
      servers: form.servers.map((current, index) => (index === selectedIndex ? server : current)),
    });
  };

  const addServer = () => {
    setActiveServerIndex(form.servers.length);
    onUpdate({ servers: [...form.servers, emptyLspServer()] });
  };

  const removeServer = (index: number) => {
    const servers = form.servers.filter((_, currentIndex) => currentIndex !== index);
    setActiveServerIndex(Math.max(0, Math.min(index, servers.length - 1)));
    onUpdate({ servers });
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <div className="settings-form-wrap">
        <FeatureAvailabilityNotice
          featureName="Language Server Protocol"
          availability={availability}
          error={availabilityError}
        />
      </div>
      <fieldset disabled={unavailable} className="contents">
        <div className="settings-form-wrap">
          <SettingsGroup title="LSP Lifecycle">
            <SettingsItem title="Enable LSP Code Intelligence">
              <Switch checked={form.enabled} onCheckedChange={(enabled) => onUpdate({ enabled })} />
            </SettingsItem>
            <SettingsItem title="Request Timeout (ms)">
              <Input numeric type="number" min={1} className="w-[120px]" value={form.request_timeout_ms} onChange={(event) => onUpdate({ request_timeout_ms: Math.max(1, Number(event.target.value) || 15000) })} />
            </SettingsItem>
            <SettingsItem title="Maximum Message Bytes">
              <Input numeric type="number" min={1024} className="w-[140px]" value={form.max_message_bytes} onChange={(event) => onUpdate({ max_message_bytes: Math.max(1024, Number(event.target.value) || 8388608) })} />
            </SettingsItem>
            <SettingsItem title="Maximum Restarts">
              <Input numeric type="number" min={0} className="w-[100px]" value={form.max_restarts} onChange={(event) => onUpdate({ max_restarts: Math.max(0, Number(event.target.value) || 0) })} />
            </SettingsItem>
            <SettingsItem title="Restart Base Delay (ms)">
              <Input numeric type="number" min={1} className="w-[120px]" value={form.restart_base_delay_ms} onChange={(event) => onUpdate({ restart_base_delay_ms: Math.max(1, Number(event.target.value) || 250) })} />
            </SettingsItem>
          </SettingsGroup>
        </div>

        <SubListLayout
          sidebar={
            <>
              <div className="sub-list-items">
                {form.servers.map((server, index) => (
                  <button
                    key={`${server.id}-${index}`}
                    type="button"
                    className={`sub-list-item ${selectedIndex === index ? 'active' : ''}`}
                    onClick={() => setActiveServerIndex(index)}
                  >
                    <span className="sub-list-item-label">{server.id || `Server ${index + 1}`}</span>
                    <span
                      className="sub-list-item-close"
                      role="button"
                      tabIndex={0}
                      title="Remove server"
                      onClick={(event) => { event.stopPropagation(); removeServer(index); }}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter') {
                          event.stopPropagation();
                          removeServer(index);
                        }
                      }}
                    >
                      <X size={11} />
                    </span>
                  </button>
                ))}
              </div>
              <Button type="button" variant="ghost" size="sm" onClick={addServer} className="w-full">
                <Plus size={13} />
                Add server
              </Button>
            </>
          }
        >
          {selectedServer ? (
            <LspServerFields server={selectedServer} onChange={changeServer} />
          ) : (
            <div className="settings-empty">No language servers configured.</div>
          )}
        </SubListLayout>
      </fieldset>
    </div>
  );
}

interface LspTabProps {
  loadSection: (section: string) => Promise<string>;
  form: LspFormData;
  setForm: React.Dispatch<React.SetStateAction<LspFormData>>;
  setDirty: React.Dispatch<React.SetStateAction<boolean>>;
  setRawToml: React.Dispatch<React.SetStateAction<string | undefined>>;
  availability: RuntimeFeatureAvailability | null | undefined;
  availabilityError?: string | null;
}

export function LspTab({
  loadSection,
  form,
  setForm,
  setDirty,
  setRawToml,
  availability,
  availabilityError,
}: LspTabProps) {
  const settings = useSettingsTab({
    section: 'lsp',
    schema: LSP_SCHEMA,
    configKey: 'lsp',
    form,
    setForm,
    setDirty,
    setRawToml,
    jsonToForm: jsonToLsp,
    loadSection,
  });

  return (
    <SettingsTabShell
      loading={settings.loading}
      rawMode={settings.rawMode}
      rawContent={settings.rawContent}
      onToggleRaw={settings.handleToggleRaw}
      onRawChange={settings.handleRawChange}
      rawPlaceholder="No lsp.toml found. Content will be created on save."
      editingDisabled={!availability?.available}
      form={
        <LspSettingsFields
          form={form}
          availability={availability}
          availabilityError={availabilityError}
          onUpdate={settings.update}
        />
      }
    />
  );
}
