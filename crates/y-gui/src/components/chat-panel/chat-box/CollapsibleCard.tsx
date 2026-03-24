/**
 * CollapsibleCard -- generic collapsible container primitive.
 *
 * Shared by ThinkingCard, ActionCard, and ToolCallCard. Renders a clickable
 * header with icon, label, optional right-side content, and a chevron. The
 * body is shown/hidden via the controlled `expanded` prop.
 */

import type { ReactNode, CSSProperties } from 'react';
import { ChevronRight } from 'lucide-react';
import './CollapsibleCard.css';

interface CollapsibleCardProps {
  /** CSS color for the left border accent and icon. */
  accentColor?: string;
  /** Icon displayed at the left of the header. */
  icon: ReactNode;
  /** Primary label text/node in the header. */
  label: ReactNode;
  /** Whether the body is expanded (controlled). */
  expanded: boolean;
  /** Toggle callback invoked when header is clicked. */
  onToggle: () => void;
  /** Optional content rendered between the label and chevron (e.g. status, duration). */
  headerRight?: ReactNode;
  /** Collapsible body content. */
  children?: ReactNode;
  /** Additional CSS class on the root element. */
  className?: string;
}

export function CollapsibleCard({
  accentColor,
  icon,
  label,
  expanded,
  onToggle,
  headerRight,
  children,
  className,
}: CollapsibleCardProps) {
  const style: CSSProperties | undefined = accentColor
    ? ({ '--collapsible-accent': accentColor } as CSSProperties)
    : undefined;

  return (
    <div className={`collapsible-card${className ? ` ${className}` : ''}`} style={style}>
      <div className="collapsible-card-header" onClick={onToggle}>
        <span className="collapsible-card-icon">{icon}</span>
        <span className="collapsible-card-label">{label}</span>
        {headerRight && (
          <span className="collapsible-card-right">{headerRight}</span>
        )}
        <span className={`collapsible-card-chevron ${expanded ? 'expanded' : ''}`}>
          <ChevronRight size={12} />
        </span>
      </div>
      {expanded && children && (
        <div className="collapsible-card-body">{children}</div>
      )}
    </div>
  );
}
