// ---------------------------------------------------------------------------
// AboutTab -- Application information
// ---------------------------------------------------------------------------

import { useState, useEffect } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { openUrl } from '@tauri-apps/plugin-opener';

export function AboutTab() {
  const [appVersion, setAppVersion] = useState('...');

  useEffect(() => {
    getVersion().then((v) => setAppVersion(v)).catch(() => setAppVersion('unknown'));
  }, []);

  return (
    <div className="settings-section">
      <h3 className="section-title">y-agent Desktop</h3>
      <div className="about-info">
        <div className="about-row">
          <span className="about-label">Author</span>
          <span className="about-value"><a href="#" onClick={(e) => { e.preventDefault(); openUrl('https://gorgias.me'); }}>Gorgias</a></span>
        </div>
        <div className="about-row">
          <span className="about-label">Version</span>
          <span className="about-value">{appVersion}</span>
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
  );
}
