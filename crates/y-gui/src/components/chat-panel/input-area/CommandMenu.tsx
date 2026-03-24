import { useState, useEffect, useRef, useMemo } from 'react';
import { Search, Puzzle, BookOpen } from 'lucide-react';
import { filterCommands, CATEGORY_ORDER } from '../../../commands';
import type { GuiCommandDef, CommandCategory } from '../../../commands';
import type { SkillInfo, KnowledgeCollectionInfo } from '../../../types';
import './CommandMenu.css';

/** Union item type for the flat navigation list. */
type MenuItem =
  | { kind: 'command'; command: GuiCommandDef }
  | { kind: 'skill'; skill: SkillInfo }
  | { kind: 'collection'; collection: KnowledgeCollectionInfo };

interface CommandMenuProps {
  skills: SkillInfo[];
  knowledgeCollections?: KnowledgeCollectionInfo[];
  onSelect: (command: GuiCommandDef) => void;
  onSelectSkill: (skillName: string) => void;
  onSelectKbCollection?: (collectionName: string) => void;
  onDismiss: () => void;
}

export function CommandMenu({
  skills,
  knowledgeCollections = [],
  onSelect,
  onSelectSkill,
  onSelectKbCollection,
  onDismiss,
}: CommandMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [search, setSearch] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);

  // Focus search input on mount.
  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  // Filter commands, skills, and knowledge collections by search query.
  const filteredCommands = useMemo(() => filterCommands(search), [search]);
  const filteredSkills = useMemo(() => {
    if (!search) return skills;
    const q = search.toLowerCase();
    return skills.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some((t) => t.toLowerCase().includes(q)),
    );
  }, [search, skills]);
  const filteredCollections = useMemo(() => {
    if (!search) return knowledgeCollections;
    const q = search.toLowerCase();
    return knowledgeCollections.filter(
      (c) =>
        c.name.toLowerCase().includes(q) ||
        c.description.toLowerCase().includes(q),
    );
  }, [search, knowledgeCollections]);

  // Build a flat item list for keyboard navigation.
  const flatItems = useMemo<MenuItem[]>(() => {
    const items: MenuItem[] = [];
    // Commands grouped by category, preserving category order.
    for (const cat of CATEGORY_ORDER) {
      for (const cmd of filteredCommands) {
        if (cmd.category === cat) {
          items.push({ kind: 'command', command: cmd });
        }
      }
    }
    // Skills follow after commands.
    for (const skill of filteredSkills) {
      items.push({ kind: 'skill', skill });
    }
    // Knowledge collections follow after skills.
    for (const collection of filteredCollections) {
      items.push({ kind: 'collection', collection });
    }
    return items;
  }, [filteredCommands, filteredSkills, filteredCollections]);

  // Clamp selection.
  const clampedIndex = Math.max(0, Math.min(selectedIndex, flatItems.length - 1));

  // Reset selection when search changes.
  useEffect(() => {
    setSelectedIndex(0);
  }, [search]);

  // Scroll selected item into view.
  useEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const selected = menu.querySelector('.command-menu-item.selected');
    if (selected) {
      selected.scrollIntoView({ block: 'nearest' });
    }
  }, [clampedIndex]);

  // Dismiss on outside click.
  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onDismiss();
      }
    };
    document.addEventListener('mousedown', handleMouseDown);
    return () => document.removeEventListener('mousedown', handleMouseDown);
  }, [onDismiss]);

  // Select the current item.
  const selectItem = (item: MenuItem) => {
    if (item.kind === 'command') {
      onSelect(item.command);
    } else if (item.kind === 'skill') {
      onSelectSkill(item.skill.name);
    } else {
      onSelectKbCollection?.(item.collection.name);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(0, i - 1));
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(flatItems.length - 1, i + 1));
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      onDismiss();
      return;
    }
    if (e.key === 'Enter' || e.key === 'Tab') {
      e.preventDefault();
      if (flatItems.length > 0) {
        selectItem(flatItems[clampedIndex]);
      }
      return;
    }
  };

  // Build grouped display structure for commands.
  const commandGroups: { category: CommandCategory; items: { cmd: GuiCommandDef; flatIndex: number }[] }[] = [];
  let flatIndex = 0;
  for (const cat of CATEGORY_ORDER) {
    const items: { cmd: GuiCommandDef; flatIndex: number }[] = [];
    for (const cmd of filteredCommands) {
      if (cmd.category === cat) {
        items.push({ cmd, flatIndex });
        flatIndex++;
      }
    }
    if (items.length > 0) {
      commandGroups.push({ category: cat, items });
    }
  }

  // Skills flat index starts after commands.
  const skillsStartIndex = flatIndex;
  // Collections flat index starts after skills.
  const collectionsStartIndex = skillsStartIndex + filteredSkills.length;

  return (
    <div className="command-menu" ref={menuRef} onKeyDown={handleKeyDown}>
      {/* Search input */}
      <div className="command-menu-search">
        <Search size={14} className="command-menu-search-icon" />
        <input
          ref={searchRef}
          type="text"
          className="command-menu-search-input"
          placeholder="Search commands, skills, and collections..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {/* Scrollable list area */}
      <div className="command-menu-list">
        {flatItems.length === 0 ? (
          <div className="command-menu-empty">No matching results</div>
        ) : (
          <>
            {/* Commands section */}
            {commandGroups.length > 0 && (
              <div className="command-menu-section">
                <div className="command-menu-section-label">Commands</div>
                {commandGroups.map((group) => (
                  <div key={group.category} className="command-menu-group">
                    <div className="command-menu-category">{group.category}</div>
                    {group.items.map(({ cmd, flatIndex: idx }) => (
                      <div
                        key={cmd.name}
                        className={`command-menu-item ${idx === clampedIndex ? 'selected' : ''}`}
                        onMouseEnter={() => setSelectedIndex(idx)}
                        onMouseDown={(e) => {
                          e.preventDefault();
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
            )}

            {/* Skills section */}
            {filteredSkills.length > 0 && (
              <div className="command-menu-section">
                <div className="command-menu-section-label">Skills</div>
                {filteredSkills.map((skill, i) => {
                  const idx = skillsStartIndex + i;
                  return (
                    <div
                      key={skill.name}
                      className={`command-menu-item ${idx === clampedIndex ? 'selected' : ''}`}
                      onMouseEnter={() => setSelectedIndex(idx)}
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onSelectSkill(skill.name);
                      }}
                    >
                      <Puzzle size={13} className="command-menu-skill-icon" />
                      <span className="command-menu-name">{skill.name}</span>
                      <span className="command-menu-desc">{skill.description}</span>
                    </div>
                  );
                })}
              </div>
            )}

            {/* Knowledge Collections section */}
            {filteredCollections.length > 0 && (
              <div className="command-menu-section">
                <div className="command-menu-section-label">Knowledge</div>
                {filteredCollections.map((collection, i) => {
                  const idx = collectionsStartIndex + i;
                  return (
                    <div
                      key={collection.id}
                      className={`command-menu-item ${idx === clampedIndex ? 'selected' : ''}`}
                      onMouseEnter={() => setSelectedIndex(idx)}
                      onMouseDown={(e) => {
                        e.preventDefault();
                        onSelectKbCollection?.(collection.name);
                      }}
                    >
                      <BookOpen size={13} className="command-menu-skill-icon" />
                      <span className="command-menu-name">#{collection.name}</span>
                      <span className="command-menu-desc">
                        {collection.entry_count} entries · {collection.description || 'Knowledge collection'}
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
