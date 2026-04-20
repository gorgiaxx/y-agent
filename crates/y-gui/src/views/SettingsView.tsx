import { SettingsPanel } from '../components/settings/SettingsPanel';
import { useConfigContext, useProvidersContext, useViewRouting } from '../providers/AppContexts';

export function SettingsView() {
  const configHooks = useConfigContext();
  const providerHooks = useProvidersContext();
  const viewRouting = useViewRouting();

  return (
    <SettingsPanel
      config={configHooks.config}
      activeTab={viewRouting.activeSettingsTab}
      onSave={async (updates) => {
        await configHooks.updateConfig(updates);
        providerHooks.refreshProviders();
        providerHooks.refreshProviderIcons();
      }}
      loadSection={configHooks.loadSection}
      saveSection={configHooks.saveSection}
      reloadConfig={async () => {
        await configHooks.reloadConfig();
        providerHooks.refreshProviders();
        return '';
      }}
      onRunWizard={viewRouting.onRunWizard}
    />
  );
}
