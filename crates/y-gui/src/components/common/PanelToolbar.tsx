import type { ReactNode } from 'react';
import {
  ChevronsDownUp,
  ChevronsUpDown,
  ArrowUpDown,
  ChevronRight,
  ChevronDown,
} from 'lucide-react';
import {
  Select,
  SelectTrigger,
  SelectContent,
  SelectItem,
  SelectValue,
} from '../ui';
import './PanelToolbar.css';

export type SortField = 'default' | 'name' | 'created' | 'updated';

export interface PanelToolbarAction {
  icon: ReactNode;
  title: string;
  onClick: (e: React.MouseEvent) => void;
}

export interface PanelToolbarProps {
  label: string;
  sortValue: SortField;
  onSortChange: (value: SortField) => void;
  collapseToggle?: { allExpanded: boolean; onToggle: () => void };
  collapsed?: boolean;
  onCollapseToggle?: () => void;
  actions?: PanelToolbarAction[];
}

const SORT_OPTIONS: { value: SortField; label: string }[] = [
  { value: 'default', label: 'Custom' },
  { value: 'name', label: 'Name' },
  { value: 'created', label: 'Created' },
  { value: 'updated', label: 'Last Active' },
];

export function PanelToolbar({
  label,
  sortValue,
  onSortChange,
  collapseToggle,
  collapsed,
  onCollapseToggle,
  actions,
}: PanelToolbarProps) {
  const isCollapsed = collapsed === true;

  return (
    <div className="panel-toolbar">
      <div className="panel-toolbar-label-wrap">
        <span className="panel-toolbar-label">{label}</span>
        {onCollapseToggle != null && (
          <button
            className="panel-toolbar-collapse-btn"
            onClick={onCollapseToggle}
            title={isCollapsed ? 'Expand' : 'Collapse'}
          >
            {isCollapsed
              ? <ChevronRight size={12} />
              : <ChevronDown size={12} />}
          </button>
        )}
      </div>
      {!isCollapsed && (
        <div className="panel-toolbar-actions">
          <Select value={sortValue} onValueChange={(v) => onSortChange(v as SortField)}>
            <SelectTrigger
              className="panel-toolbar-sort-trigger"
              title="Sort by"
            >
              <ArrowUpDown size={12} />
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {SORT_OPTIONS.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  {opt.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {collapseToggle && (
            <button
              className="panel-toolbar-btn"
              onClick={collapseToggle.onToggle}
              title={collapseToggle.allExpanded ? 'Collapse All' : 'Expand All'}
            >
              {collapseToggle.allExpanded
                ? <ChevronsDownUp size={13} />
                : <ChevronsUpDown size={13} />}
            </button>
          )}
          {actions?.map((action, i) => (
            <button
              key={i}
              className="panel-toolbar-btn"
              onClick={action.onClick}
              title={action.title}
            >
              {action.icon}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
