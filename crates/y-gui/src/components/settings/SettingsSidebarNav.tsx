import type { ReactElement } from 'react';
import {
  Settings,
  Server,
  MessageSquare,
  Terminal,
  Globe,
  Cable,
  HardDrive,
  Webhook,
  Wrench,
  Shield,
  BookOpen,
  FileText,
  Info,
} from 'lucide-react';

interface SettingsSidebarNavProps {
  activeTab: string | null;
  onSelectTab: (tab: string) => void;
}

const SETTINGS_ICON_MAP: Record<string, (props: { size: number; className: string }) => ReactElement> = {
  general:    (p) => <Settings {...p} />,
  providers:  (p) => <Server {...p} />,
  session:    (p) => <MessageSquare {...p} />,
  runtime:    (p) => <Terminal {...p} />,
  browser:    (p) => <Globe {...p} />,
  mcp:        (p) => <Cable {...p} />,
  storage:    (p) => <HardDrive {...p} />,
  hooks:      (p) => <Webhook {...p} />,
  tools:      (p) => <Wrench {...p} />,
  guardrails: (p) => <Shield {...p} />,
  knowledge:  (p) => <BookOpen {...p} />,
  prompts:    (p) => <FileText {...p} />,
  about:      (p) => <Info {...p} />,
};

const SETTINGS_CATEGORIES: { key: string; label: string }[] = [
  { key: 'general', label: 'General' },
  { key: 'providers', label: 'Providers' },
  { key: 'session', label: 'Session' },
  { key: 'runtime', label: 'Runtime' },
  { key: 'browser', label: 'Browser' },
  { key: 'mcp', label: 'MCP Servers' },
  { key: 'storage', label: 'Storage' },
  { key: 'hooks', label: 'Hooks' },
  { key: 'tools', label: 'Tools' },
  { key: 'guardrails', label: 'Guardrails' },
  { key: 'knowledge', label: 'Knowledge' },
  { key: 'prompts', label: 'Builtin Prompts' },
  { key: 'about', label: 'About' },
];

export function SettingsSidebarNav({ activeTab, onSelectTab }: SettingsSidebarNavProps) {
  return (
    <div className="sidebar-list">
      {SETTINGS_CATEGORIES.map((cat) => {
        const IconFn = SETTINGS_ICON_MAP[cat.key];
        const icon = IconFn
          ? IconFn({ size: 14, className: 'sidebar-item-icon' })
          : <Settings size={14} className="sidebar-item-icon" />;
        return (
          <div
            key={cat.key}
            className={`sidebar-item ${activeTab === cat.key ? 'active' : ''}`}
            onClick={() => onSelectTab(cat.key)}
          >
            <div className="sidebar-item-header">
              {icon}
              <span className="sidebar-item-name">{cat.label}</span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
