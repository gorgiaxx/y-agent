import { Scissors } from 'lucide-react';
import './ContextResetDivider.css';

export function ContextResetDivider() {
  return (
    <div className="context-reset-divider" role="separator">
      <span className="context-reset-divider-line" />
      <span className="context-reset-divider-label">
        <Scissors size={12} />
        <span>Context reset -- messages above are not sent to the model</span>
      </span>
      <span className="context-reset-divider-line" />
    </div>
  );
}
