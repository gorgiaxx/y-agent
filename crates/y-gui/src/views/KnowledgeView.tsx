import { KnowledgePanel } from '../components/knowledge/KnowledgePanel';
import { useKnowledgeContext } from '../providers/AppContexts';

export function KnowledgeView() {
  const knowledgeHooks = useKnowledgeContext();

  return (
    <KnowledgePanel
      collections={knowledgeHooks.collections}
      entries={knowledgeHooks.entries}
      selectedCollection={knowledgeHooks.selectedCollection}
      onSelectCollection={knowledgeHooks.setSelectedCollection}
      onCreateCollection={knowledgeHooks.createCollection}
      onDeleteCollection={knowledgeHooks.deleteCollection}
      onRenameCollection={knowledgeHooks.renameCollection}
      onGetEntryDetail={knowledgeHooks.getEntryDetail}
      onDeleteEntry={knowledgeHooks.deleteEntry}
      onSearch={knowledgeHooks.search}
      onIngestBatch={knowledgeHooks.ingestBatch}
    />
  );
}
