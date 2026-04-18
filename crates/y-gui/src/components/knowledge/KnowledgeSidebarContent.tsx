import { useKnowledgeContext, useNavigationContext } from '../../providers/AppContexts';
import { KnowledgeSidebarPanel } from './KnowledgeSidebarPanel';

export function KnowledgeSidebarContent() {
  const knowledgeHooks = useKnowledgeContext();
  const navProps = useNavigationContext();

  return (
    <KnowledgeSidebarPanel
      collections={knowledgeHooks.collections}
      selectedCollection={navProps.selectedKbCollection}
      onSelectCollection={navProps.setSelectedKbCollection}
      onCreateCollection={knowledgeHooks.createCollection}
      kbIngestStatus={knowledgeHooks.ingestStatus}
      kbBatchProgress={knowledgeHooks.batchProgress}
      kbIngestError={knowledgeHooks.ingestError}
      onClearKbIngestStatus={knowledgeHooks.clearIngestStatus}
      onCancelKbIngest={knowledgeHooks.cancelIngest}
    />
  );
}
