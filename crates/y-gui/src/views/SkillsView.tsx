import { SkillsPanel } from '../components/skills/SkillsPanel';
import { SkillImportDialog } from '../components/skills/SkillImportDialog';
import { SkillsSidebarPanel } from '../components/skills/SkillsSidebarPanel';
import { NavSidebar } from '../components/common/NavSidebar';
import { useSkillsContext, useNavigationContext } from '../providers/AppContexts';

export function SkillsView() {
  const skillHooks = useSkillsContext();
  const navProps = useNavigationContext();

  return (
    <div className="view-shell">
      <NavSidebar bare>
        <SkillsSidebarPanel
          skills={skillHooks.skills}
          activeSkillName={navProps.activeSkillName}
          importStatus={skillHooks.importStatus}
          importError={skillHooks.importError}
          onSelectSkill={navProps.setActiveSkillName}
          onImportClick={() => navProps.setImportDialogOpen(true)}
          onClearImportStatus={skillHooks.clearImportStatus}
        />
      </NavSidebar>

      <section className="view-main-pane">
        <SkillsPanel
          skillName={navProps.activeSkillName}
          onGetDetail={skillHooks.getSkillDetail}
          onGetFiles={skillHooks.getSkillFiles}
          onReadFile={skillHooks.readSkillFile}
          onSaveFile={skillHooks.saveSkillFile}
          onUninstall={async (name) => {
            await skillHooks.uninstallSkill(name);
            navProps.setActiveSkillName(null);
          }}
          onSetEnabled={async (name, enabled) => {
            await skillHooks.setEnabled(name, enabled);
          }}
          onOpenFolder={skillHooks.openFolder}
        />
      </section>

      {navProps.importDialogOpen && (
        <SkillImportDialog
          onClose={() => navProps.setImportDialogOpen(false)}
          onImport={(path, sanitize) => {
            skillHooks.importSkill(path, sanitize);
          }}
        />
      )}
    </div>
  );
}
