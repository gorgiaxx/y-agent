import { useEffect, useState, useCallback } from 'react';
import { platform, transport } from './lib';
import { GlobalProviders } from './providers/GlobalProviders';
import { MainLayout } from './layouts/MainLayout';
import { SetupWizard } from './components/wizard/SetupWizard';
import { ToastProvider } from './components/ui/Toast';
import { TooltipProvider } from './components/ui/Tooltip';
import { useConfigContext, useProvidersContext } from './providers/AppContexts';
import { resolveHostDataset } from './lib/hostDataset';
import './App.css';

function AppContent({ onRequestWizard }: { onRequestWizard: () => void }) {
  const configHooks = useConfigContext();
  const providerHooks = useProvidersContext();
  const { config, updateConfig, loading } = configHooks;
  const [checked, setChecked] = useState(false);

  // Show the window once React tree is mounted (prevents white-flash).
  useEffect(() => {
    transport.invoke('show_window').catch(() => {});
  }, []);

  // Keep the native decoration state in sync with the user preference.
  // Also expose it as a class on <html> so CSS can adapt the custom titlebar
  // (e.g. reserve space for macOS traffic lights, enable the drag region).
  useEffect(() => {
    const useCustom = !!config.use_custom_decorations;
    const platformDataset = resolveHostDataset(
      platform.isTauri(),
      typeof navigator !== 'undefined' ? navigator.platform : undefined,
    );
    document.documentElement.classList.toggle('custom-decorations', useCustom);
    document.documentElement.dataset.host = platformDataset.host;
    document.documentElement.dataset.platform = platformDataset.platform;
    transport.invoke('window_set_decorations', { useCustom }).catch(() => {});
  }, [config.use_custom_decorations]);

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

  useEffect(() => {
    const sendPong = () => transport.invoke('heartbeat_pong').catch(() => {});
    sendPong();
    let timer = setInterval(sendPong, 15_000);

    const onVisibility = () => {
      if (document.visibilityState === 'visible') {
        sendPong();
        clearInterval(timer);
        timer = setInterval(sendPong, 15_000);
      }
    };
    document.addEventListener('visibilitychange', onVisibility);
    const onFocus = () => sendPong();
    window.addEventListener('focus', onFocus);

    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('focus', onFocus);
    };
  }, []);

  const handleRunWizard = useCallback(() => {
    setShowWizard(true);
  }, []);

  const handleWizardClose = useCallback(() => {
    setShowWizard(false);
  }, []);

  return (
    <TooltipProvider>
      <ToastProvider>
        <GlobalProviders onRunWizard={handleRunWizard}>
          {showWizard ? (
            <WizardWrapper onComplete={handleWizardClose} />
          ) : (
            <AppContent onRequestWizard={handleRunWizard} />
          )}
        </GlobalProviders>
      </ToastProvider>
    </TooltipProvider>
  );
}

// Thin wrapper to read context inside GlobalProviders
function WizardWrapper({ onComplete }: { onComplete: () => void }) {
  const configHooks = useConfigContext();
  const { config, updateConfig, saveSection } = configHooks;

  // Show the window (may be first render).
  useEffect(() => {
    transport.invoke('show_window').catch(() => {});
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
