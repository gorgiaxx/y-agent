import type { ReactElement } from 'react';
import {
  Settings,
  Wrench,
  Sparkles,
  BookOpen,
  MessageSquare,
  Cpu,
  Gauge,
  ArrowLeft,
  Cable,
} from 'lucide-react';
import { NavSidebar, NavItem, NavDivider } from '../common/NavSidebar';
import type { EditorTab, EditorSurface } from '../agents/types';
import { EDITOR_TABS } from '../agents/types';
import { Switch } from '../ui/Switch';

interface AgentEditorSidebarNavProps {
  activeTab: EditorTab;
  surface: EditorSurface;
  onSelectTab: (tab: EditorTab) => void;
  onSurfaceChange: (surface: EditorSurface) => void;
  onBack: () => void;
}

const TAB_ICON_MAP: Record<EditorTab, (props: { size: number }) => ReactElement> = {
  general:   (p) => <Settings {...p} />,
  tools:     (p) => <Wrench {...p} />,
  skills:    (p) => <Sparkles {...p} />,
  knowledge: (p) => <BookOpen {...p} />,
  prompt:    (p) => <MessageSquare {...p} />,
  model:     (p) => <Cpu {...p} />,
  limits:    (p) => <Gauge {...p} />,
  mcp:       (p) => <Cable {...p} />,
};

export function AgentEditorSidebarNav({
  activeTab,
  surface,
  onSelectTab,
  onSurfaceChange,
  onBack,
}: AgentEditorSidebarNavProps) {
  return (
    <NavSidebar
      footer={
        <label className="raw-mode-switch" title={surface === 'raw' ? 'Switch to Form view' : 'Switch to Raw TOML view'}>
          <span className={`raw-mode-switch-label ${surface === 'raw' ? '' : 'raw-mode-switch-label--active'}`}>Form</span>
          <Switch
            checked={surface === 'raw'}
            onCheckedChange={(checked) => void onSurfaceChange(checked ? 'raw' : 'form')}
            aria-label="Toggle RAW mode"
          />
          <span className={`raw-mode-switch-label ${surface === 'raw' ? 'raw-mode-switch-label--active' : ''}`}>RAW</span>
        </label>
      }
    >
      <NavItem
        icon={<ArrowLeft size={15} />}
        label="Back"
        onClick={onBack}
      />
      <NavDivider />
      {EDITOR_TABS.map((item) => {
        const iconFn = TAB_ICON_MAP[item.id];
        const icon = iconFn ? iconFn({ size: 15 }) : <Settings size={15} />;
        const isActive = activeTab === item.id && surface === 'form';
        return (
          <NavItem
            key={item.id}
            icon={icon}
            label={item.label}
            active={isActive}
            onClick={() => {
              if (surface === 'raw') {
                void onSurfaceChange('form');
              }
              onSelectTab(item.id);
            }}
          />
        );
      })}
    </NavSidebar>
  );
}
