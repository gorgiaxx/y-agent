import { useState, useCallback } from 'react';
import { ContextMenu, type ContextMenuItem } from './ContextMenu';

export type { ContextMenuItem };

export function useContextMenu() {
  const [menu, setMenu] = useState<{ items: ContextMenuItem[]; position: { x: number; y: number } } | null>(null);

  const show = useCallback((e: React.MouseEvent, items: ContextMenuItem[]) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ items, position: { x: e.clientX, y: e.clientY } });
  }, []);

  const close = useCallback(() => setMenu(null), []);

  const rendered = menu
    ? <ContextMenu items={menu.items} position={menu.position} onClose={close} />
    : null;

  return { show, close, rendered };
}
