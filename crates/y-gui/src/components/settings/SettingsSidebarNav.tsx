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
import { ScrollArea } from '../ui';

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
    <ScrollArea className="flex-1">
      <div className="px-2 py-1">
        {SETTINGS_CATEGORIES.map((cat) => {
          const IconFn = SETTINGS_ICON_MAP[cat.key];
          const isActive = activeTab === cat.key;
          const icon = IconFn
            ? IconFn({ size: 14, className: `shrink-0 ${isActive ? 'text-[var(--accent)]' : 'text-[var(--text-muted)]'}` })
            : <Settings size={14} className="shrink-0 text-[var(--text-muted)]" />;
          return (
            <div
              key={cat.key}
              className={[
                'flex items-center gap-1.5',
                'px-2 py-2.5',
                'rounded-[var(--radius-md)]',
                'cursor-pointer',
                'transition-colors duration-120',
                'mb-0.5',
                'border border-solid',
                isActive
                  ? 'bg-[rgba(255,255,255,0.06)] border-[rgba(255,255,255,0.06)]'
                  : 'border-transparent hover:bg-[var(--surface-hover)]',
              ].join(' ')}
              onClick={() => onSelectTab(cat.key)}
            >
              {icon}
              <span className={[
                'text-13px font-600',
                'flex-1 min-w-0',
                'whitespace-nowrap overflow-hidden text-ellipsis',
                isActive ? 'text-[var(--text-primary)]' : 'text-[var(--text-primary)]',
              ].join(' ')}>
                {cat.label}
              </span>
            </div>
          );
        })}
      </div>
    </ScrollArea>
  );
}
