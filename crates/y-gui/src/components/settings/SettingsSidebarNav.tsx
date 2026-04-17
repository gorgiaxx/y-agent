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
import { NavSidebar, NavItem } from '../common/NavSidebar';

interface SettingsSidebarNavProps {
  activeTab: string | null;
  onSelectTab: (tab: string) => void;
}

const SETTINGS_ICON_MAP: Record<string, (props: { size: number }) => ReactElement> = {
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
    <NavSidebar variant="narrow">
      {SETTINGS_CATEGORIES.map((cat) => {
        const iconFn = SETTINGS_ICON_MAP[cat.key];
        const icon = iconFn ? iconFn({ size: 15 }) : <Settings size={15} />;
        return (
          <NavItem
            key={cat.key}
            icon={icon}
            label={cat.label}
            active={activeTab === cat.key}
            onClick={() => onSelectTab(cat.key)}
          />
        );
      })}
    </NavSidebar>
  );
}
