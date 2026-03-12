import { useState } from 'react';
import type { GuiConfig } from '../types';
import './SettingsOverlay.css';

interface SettingsOverlayProps {
  config: GuiConfig;
  onSave: (updates: Partial<GuiConfig>) => void;
  onClose: () => void;
}

type SettingsTab = 'general' | 'providers' | 'about';

export function SettingsOverlay({ config, onSave, onClose }: SettingsOverlayProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>('general');
  const [localConfig, setLocalConfig] = useState<GuiConfig>({ ...config });

  const tabs: { key: SettingsTab; label: string; icon: string }[] = [
    { key: 'general', label: 'General', icon: '⚙️' },
    { key: 'providers', label: 'Providers', icon: '🔌' },
    { key: 'about', label: 'About', icon: 'ℹ️' },
  ];

  const handleSave = () => {
    onSave(localConfig);
    onClose();
  };

  return (
    <div className="settings-backdrop" onClick={onClose}>
      <div className="settings-overlay" onClick={(e) => e.stopPropagation()}>
        <div className="settings-header">
          <h2 className="settings-title">Settings</h2>
          <button className="btn-close" onClick={onClose}>×</button>
        </div>

        <div className="settings-body">
          <nav className="settings-tabs">
            {tabs.map((tab) => (
              <button
                key={tab.key}
                className={`settings-tab ${activeTab === tab.key ? 'active' : ''}`}
                onClick={() => setActiveTab(tab.key)}
              >
                <span className="tab-icon">{tab.icon}</span>
                <span className="tab-label">{tab.label}</span>
              </button>
            ))}
          </nav>

          <div className="settings-content">
            {activeTab === 'general' && (
              <div className="settings-section">
                <h3 className="section-title">Appearance</h3>
                
                <div className="form-group">
                  <label className="form-label">Theme</label>
                  <select
                    className="form-select"
                    value={localConfig.theme}
                    onChange={(e) =>
                      setLocalConfig({ ...localConfig, theme: e.target.value as GuiConfig['theme'] })
                    }
                  >
                    <option value="dark">Dark</option>
                    <option value="light">Light</option>
                    <option value="system">System</option>
                  </select>
                </div>

                <div className="form-group">
                  <label className="form-label">Font Size</label>
                  <div className="form-range-group">
                    <input
                      type="range"
                      className="form-range"
                      min="12"
                      max="20"
                      value={localConfig.font_size}
                      onChange={(e) =>
                        setLocalConfig({ ...localConfig, font_size: Number(e.target.value) })
                      }
                    />
                    <span className="range-value">{localConfig.font_size}px</span>
                  </div>
                </div>

                <h3 className="section-title">Behavior</h3>

                <div className="form-group">
                  <label className="form-label">
                    <input
                      type="checkbox"
                      className="form-checkbox"
                      checked={localConfig.send_on_enter}
                      onChange={(e) =>
                        setLocalConfig({ ...localConfig, send_on_enter: e.target.checked })
                      }
                    />
                    Send message on Enter
                  </label>
                  <p className="form-hint">
                    When enabled, press Enter to send and Shift+Enter for newline.
                  </p>
                </div>
              </div>
            )}

            {activeTab === 'providers' && (
              <div className="settings-section">
                <h3 className="section-title">Provider Configuration</h3>
                <p className="settings-info">
                  Provider settings are managed via configuration files at{' '}
                  <code>~/.config/y-agent/providers.toml</code>.
                </p>
                <p className="settings-info">
                  Edit this file to add, remove, or modify LLM providers.
                  Changes require an application restart to take effect.
                </p>
              </div>
            )}

            {activeTab === 'about' && (
              <div className="settings-section">
                <h3 className="section-title">y-agent Desktop</h3>
                <div className="about-info">
                  <div className="about-row">
                    <span className="about-label">Version</span>
                    <span className="about-value">0.1.0</span>
                  </div>
                  <div className="about-row">
                    <span className="about-label">Framework</span>
                    <span className="about-value">Tauri v2</span>
                  </div>
                  <div className="about-row">
                    <span className="about-label">Frontend</span>
                    <span className="about-value">React + TypeScript</span>
                  </div>
                  <div className="about-row">
                    <span className="about-label">Backend</span>
                    <span className="about-value">Rust (y-service)</span>
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>

        <div className="settings-footer">
          <button className="btn-cancel" onClick={onClose}>Cancel</button>
          <button className="btn-save" onClick={handleSave}>Save Changes</button>
        </div>
      </div>
    </div>
  );
}
