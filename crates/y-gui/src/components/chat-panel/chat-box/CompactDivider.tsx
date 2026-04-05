import { PackageMinus } from 'lucide-react';
import './CompactDivider.css';

interface CompactDividerProps {
  messagesPruned: number;
  messagesCompacted: number;
  tokensSaved: number;
}

export function CompactDivider({ messagesPruned, messagesCompacted, tokensSaved }: CompactDividerProps) {
  const parts: string[] = [];
  if (messagesPruned > 0) parts.push(`${messagesPruned} pruned`);
  if (messagesCompacted > 0) parts.push(`${messagesCompacted} compacted`);
  if (tokensSaved > 0) parts.push(`~${tokensSaved} tokens saved`);

  const detail = parts.length > 0 ? ` (${parts.join(', ')})` : '';

  return (
    <div className="compact-divider" role="separator">
      <span className="compact-divider-line" />
      <span className="compact-divider-label">
        <PackageMinus size={12} />
        <span>Context compacted{detail}</span>
      </span>
      <span className="compact-divider-line" />
    </div>
  );
}
