import { useEffect, useMemo, useRef, useState } from 'react';
import { Keyboard, Pencil, Plus, RotateCcw, Search, Trash2 } from 'lucide-react';

import type { GuiConfig } from '../../types';
import {
  GUI_SHORTCUTS,
  findShortcutConflict,
  formatShortcutBinding,
  isMacKeyboardPlatform,
  keyboardEventToBinding,
  resolveShortcutBindings,
  type GuiShortcutActionId,
} from '../../shortcuts/shortcutRegistry';

import './KeyboardShortcutsTab.css';

interface KeyboardShortcutsTabProps {
  config: GuiConfig;
  onChange: (overrides: Record<string, string[]>) => void;
}

interface EditingBinding {
  actionId: GuiShortcutActionId;
  index: number;
}

const CATEGORIES = ['General', 'Navigation', 'Panels'] as const;

function bindingsEqual(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((binding, index) => binding === right[index]);
}

export function KeyboardShortcutsTab({ config, onChange }: KeyboardShortcutsTabProps) {
  const [query, setQuery] = useState('');
  const [editing, setEditing] = useState<EditingBinding | null>(null);
  const [captureError, setCaptureError] = useState<string | null>(null);
  const captureRef = useRef<HTMLButtonElement>(null);
  const isMac = isMacKeyboardPlatform();
  const bindings = useMemo(
    () => resolveShortcutBindings(config.keyboard_shortcuts),
    [config.keyboard_shortcuts],
  );

  useEffect(() => {
    captureRef.current?.focus();
  }, [editing]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return GUI_SHORTCUTS;
    return GUI_SHORTCUTS.filter((shortcut) => {
      const searchable = [
        shortcut.title,
        shortcut.description,
        shortcut.category,
        ...bindings[shortcut.id],
      ].join(' ').toLowerCase();
      return searchable.includes(needle);
    });
  }, [bindings, query]);

  const updateBindings = (actionId: GuiShortcutActionId, nextBindings: string[]) => {
    const definition = GUI_SHORTCUTS.find((shortcut) => shortcut.id === actionId);
    if (!definition) return;
    const nextOverrides = { ...config.keyboard_shortcuts };
    if (bindingsEqual(nextBindings, definition.defaultBindings)) {
      delete nextOverrides[actionId];
    } else {
      nextOverrides[actionId] = nextBindings;
    }
    onChange(nextOverrides);
  };

  const beginCapture = (actionId: GuiShortcutActionId, index: number) => {
    setCaptureError(null);
    setEditing({ actionId, index });
  };

  const captureBinding = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    if (!editing) return;
    if (event.key === 'Escape') {
      setEditing(null);
      setCaptureError(null);
      return;
    }

    const binding = keyboardEventToBinding(event.nativeEvent, isMac);
    if (!binding) return;
    const actionBindings = bindings[editing.actionId];
    const duplicateIndex = actionBindings.findIndex((candidate) => candidate === binding);
    if (duplicateIndex >= 0 && duplicateIndex !== editing.index) {
      setCaptureError('This shortcut is already assigned to the action.');
      return;
    }
    const conflict = findShortcutConflict(binding, editing.actionId, bindings);
    if (conflict) {
      setCaptureError(`Already assigned to ${conflict.title}.`);
      return;
    }

    const next = [...actionBindings];
    if (editing.index >= next.length) next.push(binding);
    else next[editing.index] = binding;
    updateBindings(editing.actionId, next);
    setEditing(null);
    setCaptureError(null);
  };

  return (
    <div className="keyboard-shortcuts-tab">
      <label className="shortcut-search">
        <Search size={16} aria-hidden="true" />
        <input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search shortcuts"
          aria-label="Search shortcuts"
        />
        <Keyboard size={15} aria-hidden="true" />
      </label>

      <div className="shortcut-groups">
        {CATEGORIES.map((category) => {
          const shortcuts = filtered.filter((shortcut) => shortcut.category === category);
          if (shortcuts.length === 0) return null;
          return (
            <section className="shortcut-group" key={category} aria-labelledby={`shortcuts-${category}`}>
              <h3 id={`shortcuts-${category}`}>{category}</h3>
              <div className="shortcut-list">
                {shortcuts.map((shortcut) => {
                  const actionBindings = bindings[shortcut.id];
                  const customized = Object.prototype.hasOwnProperty.call(
                    config.keyboard_shortcuts,
                    shortcut.id,
                  );
                  return (
                    <div className="shortcut-row" key={shortcut.id}>
                      <div className="shortcut-copy">
                        <strong>{shortcut.title}</strong>
                        <span>{shortcut.description}</span>
                      </div>
                      <div className="shortcut-bindings">
                        {actionBindings.map((binding, index) => {
                          const isEditing = editing?.actionId === shortcut.id && editing.index === index;
                          return (
                            <div className="shortcut-binding" key={`${shortcut.id}-${index}`}>
                              {isEditing ? (
                                <button
                                  ref={captureRef}
                                  type="button"
                                  className="shortcut-capture"
                                  onKeyDown={captureBinding}
                                  onBlur={() => setEditing(null)}
                                >
                                  Press shortcut
                                </button>
                              ) : (
                                <kbd>{formatShortcutBinding(binding, isMac)}</kbd>
                              )}
                              {!isEditing && (
                                <button
                                  type="button"
                                  className="shortcut-icon-button"
                                  onClick={() => beginCapture(shortcut.id, index)}
                                  title={`Edit ${shortcut.title} shortcut`}
                                  aria-label={`Edit ${shortcut.title} shortcut`}
                                >
                                  <Pencil size={14} />
                                </button>
                              )}
                              <button
                                type="button"
                                className="shortcut-icon-button shortcut-remove"
                                onClick={() => updateBindings(
                                  shortcut.id,
                                  actionBindings.filter((_, bindingIndex) => bindingIndex !== index),
                                )}
                                title={`Remove ${shortcut.title} shortcut`}
                                aria-label={`Remove ${shortcut.title} shortcut`}
                              >
                                <Trash2 size={14} />
                              </button>
                            </div>
                          );
                        })}
                        {editing?.actionId === shortcut.id && editing.index === actionBindings.length && (
                          <button
                            ref={captureRef}
                            type="button"
                            className="shortcut-capture"
                            onKeyDown={captureBinding}
                            onBlur={() => setEditing(null)}
                          >
                            Press shortcut
                          </button>
                        )}
                        {!editing || editing.actionId !== shortcut.id ? (
                          <button
                            type="button"
                            className="shortcut-icon-button shortcut-add"
                            onClick={() => beginCapture(shortcut.id, actionBindings.length)}
                            title={`Add ${shortcut.title} shortcut`}
                            aria-label={`Add ${shortcut.title} shortcut`}
                          >
                            <Plus size={15} />
                          </button>
                        ) : null}
                        {customized && (
                          <button
                            type="button"
                            className="shortcut-icon-button shortcut-reset"
                            onClick={() => updateBindings(shortcut.id, [...shortcut.defaultBindings])}
                            title={`Reset ${shortcut.title} shortcuts`}
                            aria-label={`Reset ${shortcut.title} shortcuts`}
                          >
                            <RotateCcw size={14} />
                          </button>
                        )}
                      </div>
                      {editing?.actionId === shortcut.id && captureError && (
                        <div className="shortcut-error" role="alert">{captureError}</div>
                      )}
                    </div>
                  );
                })}
              </div>
            </section>
          );
        })}
        {filtered.length === 0 && (
          <div className="shortcut-empty">No shortcuts match "{query.trim()}".</div>
        )}
      </div>
    </div>
  );
}
