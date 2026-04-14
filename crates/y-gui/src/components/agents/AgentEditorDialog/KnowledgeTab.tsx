import { Checkbox } from '../../ui/Checkbox';
import type { AgentDraft } from '../types';
import { toggleItem } from '../utils';

interface KnowledgeTabProps {
  draft: AgentDraft;
  knowledgeCollections: string[];
  onChange: (updater: (draft: AgentDraft) => AgentDraft) => void;
}

export function KnowledgeTab({ draft, knowledgeCollections, onChange }: KnowledgeTabProps) {
  return (
    <div className="flex flex-col gap-3">
      <label className="flex items-center gap-2 cursor-pointer">
        <Checkbox
          checked={draft.knowledge_enabled}
          onCheckedChange={(checked) => onChange((prev) => ({ ...prev, knowledge_enabled: checked === true }))}
        />
        <span className="text-11px text-[var(--text-secondary)]">Enable knowledge base</span>
      </label>
      <div className="grid grid-cols-2 gap-2 max-h-[320px] overflow-y-auto">
        {knowledgeCollections.map((collection) => (
          <label
            key={collection}
            className={[
              'flex items-center gap-2 p-2 rounded-[var(--radius-sm)] border border-solid cursor-pointer',
              'transition-colors duration-150',
              draft.knowledge_collections.includes(collection)
                ? 'border-[var(--accent)] bg-[var(--accent-subtle)]'
                : 'border-[var(--border)] hover:border-[var(--border-focus)]',
            ].join(' ')}
          >
            <Checkbox
              checked={draft.knowledge_collections.includes(collection)}
              onCheckedChange={() => onChange((prev) => ({ ...prev, knowledge_collections: toggleItem(prev.knowledge_collections, collection) }))}
            />
            <div className="text-11px font-500 text-[var(--text-primary)] truncate">{collection}</div>
          </label>
        ))}
      </div>
    </div>
  );
}
