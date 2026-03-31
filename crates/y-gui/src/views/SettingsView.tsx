import { SettingsPanel } from '../components/settings/SettingsPanel';
import { useConfigContext, useProvidersContext, useNavigationContext } from '../providers/AppContexts';

export function SettingsView() {
  const configHooks = useConfigContext();
  const providerHooks = useProvidersContext();
  const navProps = useNavigationContext();

  return (
    <SettingsPanel
      config={configHooks.config}
      activeTab={navProps.activeSettingsTab}
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
    />
  );
}
