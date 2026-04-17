import { KnowledgePanel } from '../components/knowledge/KnowledgePanel';
import { KnowledgeSidebarPanel } from '../components/knowledge/KnowledgeSidebarPanel';
import { NavSidebar } from '../components/common/NavSidebar';
import { useKnowledgeContext, useNavigationContext } from '../providers/AppContexts';

export function KnowledgeView() {
  const knowledgeHooks = useKnowledgeContext();
  const navProps = useNavigationContext();

  return (
    <div className="view-shell">
      <NavSidebar bare>
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
      </NavSidebar>

      <section className="view-main-pane">
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
      </section>
    </div>
  );
}
