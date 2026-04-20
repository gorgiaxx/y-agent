import { SkillsPanel } from '../components/skills/SkillsPanel';
import { SkillImportDialog } from '../components/skills/SkillImportDialog';
import { useSkillsContext, useSkillsNavContext } from '../providers/AppContexts';

export function SkillsView() {
  const skillHooks = useSkillsContext();
  const navProps = useSkillsNavContext();

  return (
    <>
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

      {navProps.importDialogOpen && (
        <SkillImportDialog
          onClose={() => navProps.setImportDialogOpen(false)}
          onImport={(path, sanitize) => {
            skillHooks.importSkill(path, sanitize);
          }}
        />
      )}
    </>
  );
}
