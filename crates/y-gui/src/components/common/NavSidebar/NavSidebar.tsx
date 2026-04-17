/**
 * NavSidebar -- macOS Finder/Notes-style sidebar shell.
 *
 * Generic, reusable container driven by a section model. Consumers build the
 * navigation tree by composing simple primitives (items, search, custom
 * components, sections, dividers). The sidebar handles layout (scroll body +
 * pinned footer for bottom items) and styling; items' behavior stays inline.
 */
import type { ReactNode, MouseEvent } from 'react';
import { Search } from 'lucide-react';
import './NavSidebar.css';

export interface NavItemProps {
  icon?: ReactNode;
  label: ReactNode;
  active?: boolean;
  primary?: boolean;
  accessory?: ReactNode;
  title?: string;
  onClick?: (e: MouseEvent<HTMLButtonElement>) => void;
}

export function NavItem({ icon, label, active, primary, accessory, title, onClick }: NavItemProps) {
  const classes = ['nav-item'];
  if (active) classes.push('active');
  if (primary) classes.push('nav-item--primary');
  return (
    <button type="button" className={classes.join(' ')} onClick={onClick} title={title}>
      {icon !== undefined && <span className="nav-item-icon">{icon}</span>}
      <span className="nav-item-label">{label}</span>
      {accessory && <span className="nav-item-accessory">{accessory}</span>}
    </button>
  );
}

export interface NavSearchProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

export function NavSearch({ value, onChange, placeholder }: NavSearchProps) {
  return (
    <div className="nav-search">
      <Search size={12} className="nav-search-icon" />
      <input
        type="text"
        className="nav-search-input"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
      />
    </div>
  );
}

export function NavDivider() {
  return <div className="nav-divider" />;
}

export interface NavSectionHeaderProps {
  title: string;
  accessory?: ReactNode;
}

export function NavSectionHeader({ title, accessory }: NavSectionHeaderProps) {
  return (
    <div className="nav-section-header">
      <span className="nav-section-header-title">{title}</span>
      {accessory}
    </div>
  );
}

export interface NavSidebarProps {
  children: ReactNode;
  /** Items pinned to the bottom (e.g., Settings). */
  footer?: ReactNode;
  /** Narrow variant (e.g., Settings view). */
  variant?: 'default' | 'narrow';
  /**
   * When true, render children directly as flex items (no inner scroll
   * wrapper or padding). Use this for panels that manage their own header
   * and scroll region, e.g. Skills/Knowledge/Automation master lists.
   */
  bare?: boolean;
  className?: string;
}

export function NavSidebar({ children, footer, variant = 'default', bare, className }: NavSidebarProps) {
  const classes = ['nav-sidebar'];
  if (variant === 'narrow') classes.push('nav-sidebar--narrow');
  if (bare) classes.push('nav-sidebar--bare');
  if (className) classes.push(className);
  return (
    <aside className={classes.join(' ')}>
      {bare ? children : <div className="nav-sidebar-scroll">{children}</div>}
      {footer && <div className="nav-sidebar-footer">{footer}</div>}
    </aside>
  );
}
