import { useState, useRef, useEffect } from 'react';
import { ChevronDown, FolderOpen, Plus } from 'lucide-react';
import type { WorkspaceInfo } from '../types';
import './WelcomePage.css';

interface WelcomePageProps {
  workspaces: WorkspaceInfo[];
  selectedWorkspaceId: string | null;
  onSelectWorkspace: (id: string) => void;
  onCreateWorkspace: () => void;
}

export function WelcomePage({
  workspaces,
  selectedWorkspaceId,
  onSelectWorkspace,
  onCreateWorkspace,
}: WelcomePageProps) {
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  // Sort workspaces alphabetically.
  const sorted = [...workspaces].sort((a, b) =>
    a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
  );

  const selectedWorkspace = workspaces.find((w) => w.id === selectedWorkspaceId);

  // Close dropdown on outside click.
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setDropdownOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, []);

  return (
    <div className="welcome-page">
      <div className="welcome-content">
        {/* Logo */}
        <div className="welcome-logo">y</div>

        {/* Introduction */}
        <div className="welcome-intro">
          <h2 className="welcome-title">Welcome to y-agent</h2>
          <p className="welcome-description">
            An intelligent assistant powered by large language models.
            <br />
            Create sessions, manage workspaces, and leverage skills to supercharge your workflow.
          </p>
        </div>

        {/* Workspace selector */}
        <div className="welcome-workspace-selector" ref={dropdownRef}>
          <button
            className={`welcome-workspace-btn ${dropdownOpen ? 'open' : ''}`}
            onClick={() => setDropdownOpen(!dropdownOpen)}
          >
            <FolderOpen size={14} className="welcome-workspace-btn-icon" />
            <span className="welcome-workspace-btn-label">
              {selectedWorkspace ? selectedWorkspace.name : 'No workspace'}
            </span>
            <ChevronDown
              size={14}
              className={`welcome-workspace-chevron ${dropdownOpen ? 'welcome-workspace-chevron--open' : ''}`}
            />
          </button>

          {dropdownOpen && (
            <div className="welcome-workspace-dropdown">
              <div className="welcome-workspace-list">
                {sorted.map((ws) => (
                  <button
                    key={ws.id}
                    className={`welcome-workspace-item ${ws.id === selectedWorkspaceId ? 'active' : ''}`}
                    onClick={() => {
                      onSelectWorkspace(ws.id);
                      setDropdownOpen(false);
                    }}
                  >
                    <FolderOpen size={12} />
                    <span className="welcome-workspace-item-name">{ws.name}</span>
                    <span className="welcome-workspace-item-path" title={ws.path}>
                      {ws.path}
                    </span>
                  </button>
                ))}
                {sorted.length === 0 && (
                  <div className="welcome-workspace-empty">No workspaces yet</div>
                )}
              </div>
              <button
                className="welcome-workspace-create"
                onClick={() => {
                  setDropdownOpen(false);
                  onCreateWorkspace();
                }}
              >
                <Plus size={12} />
                Create Workspace
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
