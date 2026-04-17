import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Minus, Square, X, Copy } from 'lucide-react';
import './WindowControls.css';

/**
 * Non-macOS custom window controls (minimize / maximize / close).
 *
 * On macOS the traffic lights are provided by the system via Tauri's
 * `titleBarStyle: "Overlay"` configuration, so this component renders
 * nothing. On Linux / Windows, it draws a chrome-style button group that
 * replaces the native caption buttons when `use_custom_decorations` is on.
 */
export function WindowControls() {
  const [isMac, setIsMac] = useState(true);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    setIsMac(typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform));

    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    win.isMaximized().then(setMaximized).catch(() => {});
    win.onResized(() => {
      win.isMaximized().then(setMaximized).catch(() => {});
    }).then((fn) => { unlisten = fn; }).catch(() => {});
    return () => { unlisten?.(); };
  }, []);

  if (isMac) return null;

  return (
    <div className="window-controls" aria-label="Window controls">
      <button
        className="window-control-btn"
        onClick={() => invoke('window_minimize').catch(() => {})}
        title="Minimize"
        aria-label="Minimize"
      >
        <Minus size={13} />
      </button>
      <button
        className="window-control-btn"
        onClick={() => invoke('window_toggle_maximize').catch(() => {})}
        title={maximized ? 'Restore' : 'Maximize'}
        aria-label={maximized ? 'Restore' : 'Maximize'}
      >
        {maximized ? <Copy size={11} /> : <Square size={11} />}
      </button>
      <button
        className="window-control-btn window-control-close"
        onClick={() => invoke('window_close').catch(() => {})}
        title="Close"
        aria-label="Close"
      >
        <X size={13} />
      </button>
    </div>
  );
}
