import { useEffect, useRef } from 'react';
import { filterCommands, CATEGORY_ORDER } from '../commands';
import type { GuiCommandDef, CommandCategory } from '../commands';
import './CommandMenu.css';

interface CommandMenuProps {
  query: string;
  selectedIndex: number;
  onSelect: (command: GuiCommandDef) => void;
  onDismiss: () => void;
  onHover: (index: number) => void;
}

export function CommandMenu({
  query,
  selectedIndex,
  onSelect,
  onDismiss,
  onHover,
}: CommandMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const filtered = filterCommands(query);

  // Clamp selectedIndex to valid range.
  const clampedIndex = Math.max(0, Math.min(selectedIndex, filtered.length - 1));

  // Scroll the selected item into view.
  useEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const selected = menu.querySelector('.command-menu-item.selected');
    if (selected) {
      selected.scrollIntoView({ block: 'nearest' });
    }
  }, [clampedIndex]);

  // Dismiss when clicking outside.
  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onDismiss();
      }
    };
    document.addEventListener('mousedown', handleMouseDown);
    return () => document.removeEventListener('mousedown', handleMouseDown);
  }, [onDismiss]);

  if (filtered.length === 0) {
    return (
      <div className="command-menu" ref={menuRef}>
        <div className="command-menu-empty">No matching commands</div>
      </div>
    );
  }

  // Group commands by category, preserving the global flat index for selection.
  const groups: { category: CommandCategory; items: { cmd: GuiCommandDef; flatIndex: number }[] }[] = [];
  let flatIndex = 0;
  for (const cat of CATEGORY_ORDER) {
    const items: { cmd: GuiCommandDef; flatIndex: number }[] = [];
    for (const cmd of filtered) {
      if (cmd.category === cat) {
        items.push({ cmd, flatIndex });
        flatIndex++;
      }
    }
    if (items.length > 0) {
      groups.push({ category: cat, items });
    }
  }

  return (
    <div className="command-menu" ref={menuRef}>
      {groups.map((group) => (
        <div key={group.category} className="command-menu-group">
          <div className="command-menu-category">{group.category}</div>
          {group.items.map(({ cmd, flatIndex: idx }) => (
            <div
              key={cmd.name}
              className={`command-menu-item ${idx === clampedIndex ? 'selected' : ''}`}
              onMouseEnter={() => onHover(idx)}
              onMouseDown={(e) => {
                e.preventDefault(); // Prevent textarea blur.
                onSelect(cmd);
              }}
            >
              <span className="command-menu-name">/{cmd.name}</span>
              {cmd.alias && <span className="command-menu-alias">/{cmd.alias}</span>}
              <span className="command-menu-desc">{cmd.description}</span>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
