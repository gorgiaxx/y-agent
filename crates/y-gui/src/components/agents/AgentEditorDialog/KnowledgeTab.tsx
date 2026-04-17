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
    <div className="agent-editor-form-stack">
      <label className="agent-editor-checkbox-row">
        <Checkbox
          checked={draft.knowledge_enabled}
          onCheckedChange={(checked) => onChange((prev) => ({ ...prev, knowledge_enabled: checked === true }))}
        />
        <span className="agent-editor-field-label">Enable knowledge base</span>
      </label>
      <div className="agent-editor-checkbox-grid">
        {knowledgeCollections.map((collection) => (
          <label
            key={collection}
            className={[
              'agent-editor-checkbox-card agent-editor-checkbox-card--center',
              draft.knowledge_collections.includes(collection) ? 'agent-editor-checkbox-card--active' : '',
            ].join(' ')}
          >
            <Checkbox
              checked={draft.knowledge_collections.includes(collection)}
              onCheckedChange={() => onChange((prev) => ({ ...prev, knowledge_collections: toggleItem(prev.knowledge_collections, collection) }))}
            />
            <div className="agent-editor-checkbox-card-title">{collection}</div>
          </label>
        ))}
      </div>
    </div>
  );
}
