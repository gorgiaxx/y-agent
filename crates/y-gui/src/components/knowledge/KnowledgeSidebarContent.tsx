import { useKnowledgeContext } from '../../providers/AppContexts';
import { KnowledgeSidebarPanel } from './KnowledgeSidebarPanel';

export function KnowledgeSidebarContent() {
  const knowledgeHooks = useKnowledgeContext();

  return (
    <KnowledgeSidebarPanel
      collections={knowledgeHooks.collections}
      selectedCollection={knowledgeHooks.selectedCollection}
      onSelectCollection={knowledgeHooks.setSelectedCollection}
      onCreateCollection={knowledgeHooks.createCollection}
      kbIngestStatus={knowledgeHooks.ingestStatus}
      kbBatchProgress={knowledgeHooks.batchProgress}
      kbIngestError={knowledgeHooks.ingestError}
      onClearKbIngestStatus={knowledgeHooks.clearIngestStatus}
      onCancelKbIngest={knowledgeHooks.cancelIngest}
    />
  );
}
