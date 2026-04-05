import { useEffect, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GlobalProviders } from './providers/GlobalProviders';
import { MainLayout } from './layouts/MainLayout';
import { SetupWizard } from './components/wizard/SetupWizard';
import { ToastProvider } from './components/ui';
import { useConfigContext, useProvidersContext } from './providers/AppContexts';
import './App.css';

function AppContent({ onRequestWizard }: { onRequestWizard: () => void }) {
  const configHooks = useConfigContext();
  const providerHooks = useProvidersContext();
  const { config, updateConfig, loading } = configHooks;
  const [checked, setChecked] = useState(false);

  // Show the window once React tree is mounted (prevents white-flash).
  useEffect(() => {
    invoke('show_window').catch(() => {});
  }, []);

  // Determine whether to show the wizard on first load.
  // For existing users who already have providers configured, auto-mark setup
  // as completed so they never see the wizard.
  useEffect(() => {
    if (loading || checked) return;

    if (config.setup_completed) {
      setChecked(true);
      return;
    }

    // Check if providers already exist (existing user upgrading).
    if (providerHooks.providers.length > 0) {
      updateConfig({ ...config, setup_completed: true }).then(() => {
        setChecked(true);
      });
      return;
    }

    // Check if providers.toml has content (belt-and-suspenders).
    configHooks.loadSection('providers').then((toml) => {
      if (toml && toml.trim().length > 0) {
        updateConfig({ ...config, setup_completed: true }).then(() => {
          setChecked(true);
        });
      } else {
        // Truly new user -- show wizard.
        onRequestWizard();
        setChecked(true);
      }
    }).catch(() => {
      onRequestWizard();
      setChecked(true);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading, checked]);

  if (loading || !checked) return null;

  return <MainLayout />;
}

export default function App() {
  const [showWizard, setShowWizard] = useState(false);

  const handleRunWizard = useCallback(() => {
    setShowWizard(true);
  }, []);

  const handleWizardClose = useCallback(() => {
    setShowWizard(false);
  }, []);

  return (
    <ToastProvider>
      <GlobalProviders onRunWizard={handleRunWizard}>
        {showWizard ? (
          <WizardWrapper onComplete={handleWizardClose} />
        ) : (
          <AppContent onRequestWizard={handleRunWizard} />
        )}
      </GlobalProviders>
    </ToastProvider>
  );
}

// Thin wrapper to read context inside GlobalProviders
function WizardWrapper({ onComplete }: { onComplete: () => void }) {
  const configHooks = useConfigContext();
  const { config, updateConfig, saveSection } = configHooks;

  // Show the window (may be first render).
  useEffect(() => {
    invoke('show_window').catch(() => {});
  }, []);

  return (
    <SetupWizard
      config={config}
      updateConfig={updateConfig}
      saveSection={saveSection}
      onComplete={onComplete}
    />
  );
}
