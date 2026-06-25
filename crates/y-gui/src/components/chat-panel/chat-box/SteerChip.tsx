import { CornerDownRight } from 'lucide-react';
import './SteerChip.css';

interface SteerChipProps {
  /** The user's steering message text, injected mid-run. */
  text: string;
}

/**
 * SteerChip -- a compact, tool-call-tag-style chip marking a point where the
 * user steered the assistant mid-run. Rendered inline within the assistant
 * bubble's segment stream (both live streaming and persisted history) so it
 * appears at the true injection position, distinct from normal user bubbles.
 */
export function SteerChip({ text }: SteerChipProps) {
  const trimmed = text.trim();
  return (
    <div className="steer-chip" title={trimmed}>
      <span className="steer-chip-action-group">
        <CornerDownRight size={14} className="steer-chip-icon" />
        <span className="steer-chip-key">Steered</span>
      </span>
      <span className="steer-chip-text">{trimmed}</span>
    </div>
  );
}
