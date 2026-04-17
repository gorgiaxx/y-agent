import { SettingsPanel } from '../components/settings/SettingsPanel';
import { SettingsSidebarNav } from '../components/settings/SettingsSidebarNav';
import { useConfigContext, useProvidersContext, useNavigationContext } from '../providers/AppContexts';

export function SettingsView() {
  const configHooks = useConfigContext();
  const providerHooks = useProvidersContext();
  const navProps = useNavigationContext();

  return (
    <div className="view-shell">
      <SettingsSidebarNav
        activeTab={navProps.activeSettingsTab}
        onSelectTab={(t: string) => navProps.setActiveSettingsTab(t as never)}
      />

      <section className="view-main-pane">
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
          onRunWizard={navProps.onRunWizard}
        />
      </section>
    </div>
  );
}
