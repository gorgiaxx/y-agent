// ---------------------------------------------------------------------------
// AboutTab -- Application information
// ---------------------------------------------------------------------------

import { useState, useEffect } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { openUrl } from '@tauri-apps/plugin-opener';
import { SettingsGroup, SettingsItem } from '../ui';

export function AboutTab() {
  const [appVersion, setAppVersion] = useState('...');

  useEffect(() => {
    getVersion().then((v) => setAppVersion(v)).catch(() => setAppVersion('unknown'));
  }, []);

  return (
    <div className="settings-section">
      <SettingsGroup title="y-agent Desktop">
        <SettingsItem title="Author">
          <a href="#" onClick={(e) => { e.preventDefault(); openUrl('https://gorgias.me'); }}>Gorgias</a>
        </SettingsItem>
        <SettingsItem title="Version">
          <span className="about-value">{appVersion}</span>
        </SettingsItem>
        <SettingsItem title="Framework">
          <span className="about-value">Tauri v2</span>
        </SettingsItem>
        <SettingsItem title="Frontend">
          <span className="about-value">React + TypeScript</span>
        </SettingsItem>
        <SettingsItem title="Backend">
          <span className="about-value">Rust (y-service)</span>
        </SettingsItem>
      </SettingsGroup>
    </div>
  );
}
