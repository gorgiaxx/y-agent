import type { ReactNode } from 'react';
import './SubListLayout.css';

interface SubListLayoutProps {
  sidebar: ReactNode;
  children: ReactNode;
  className?: string;
}

export function SubListLayout({ sidebar, children, className }: SubListLayoutProps) {
  return (
    <div className={`sub-list-layout ${className ?? ''}`}>
      <div className="sub-list-sidebar">
        {sidebar}
      </div>
      <div className="sub-list-detail">
        {children}
      </div>
    </div>
  );
}
