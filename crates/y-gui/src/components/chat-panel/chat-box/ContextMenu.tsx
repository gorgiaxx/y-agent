import { useEffect, useRef, useLayoutEffect } from 'react';
import { createPortal } from 'react-dom';
import { isTauriEnvironment } from '../../../lib/platform';

const isMacOS = typeof navigator !== 'undefined' && /Mac/.test(navigator.platform);
const TRAFFIC_LIGHT_ZONE = { width: 80, height: 40 };

export interface ContextMenuItem {
  label: string;
  icon?: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
}

interface ContextMenuProps {
  items: ContextMenuItem[];
  position: { x: number; y: number };
  onClose: () => void;
}

export function ContextMenu({ items, position, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let x = position.x;
    let y = position.y;
    if (x + rect.width > vw) x = vw - rect.width - 8;
    if (y + rect.height > vh) y = vh - rect.height - 8;
    if (x < 0) x = 8;
    if (y < 0) y = 8;
    if (isMacOS && isTauriEnvironment()) {
      const overlapX = x < TRAFFIC_LIGHT_ZONE.width && y < TRAFFIC_LIGHT_ZONE.height;
      const overlapY = y < TRAFFIC_LIGHT_ZONE.height && x < TRAFFIC_LIGHT_ZONE.width;
      if (overlapX || overlapY) {
        y = TRAFFIC_LIGHT_ZONE.height + 4;
      }
    }
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
  }, [position]);

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('mousedown', handleClickOutside);
    document.addEventListener('keydown', handleEscape);
    return () => {
      document.removeEventListener('mousedown', handleClickOutside);
      document.removeEventListener('keydown', handleEscape);
    };
  }, [onClose]);

  return createPortal(
    <div
      ref={menuRef}
      className="md-context-menu"
      style={{ left: position.x, top: position.y }}
    >
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          className="md-context-menu-item"
          disabled={item.disabled}
          onClick={() => {
            item.onClick();
            onClose();
          }}
        >
          {item.icon && <span className="md-context-menu-icon">{item.icon}</span>}
          <span>{item.label}</span>
        </button>
      ))}
    </div>,
    document.body,
  );
}
