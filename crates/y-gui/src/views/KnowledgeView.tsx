import { KnowledgePanel } from '../components/knowledge/KnowledgePanel';
import { useKnowledgeContext, useNavigationContext } from '../providers/AppContexts';

export function KnowledgeView() {
  const knowledgeHooks = useKnowledgeContext();
  const navProps = useNavigationContext();

  return (
    <KnowledgePanel
      collections={knowledgeHooks.collections}
      entries={knowledgeHooks.entries}
      selectedCollection={navProps.selectedKbCollection}
      onSelectCollection={navProps.setSelectedKbCollection}
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
