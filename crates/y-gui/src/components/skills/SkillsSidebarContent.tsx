import { useSkillsContext, useSkillsNavContext } from '../../providers/AppContexts';
import { SkillsSidebarPanel } from './SkillsSidebarPanel';

export function SkillsSidebarContent() {
  const skillHooks = useSkillsContext();
  const navProps = useSkillsNavContext();

  return (
    <SkillsSidebarPanel
      skills={skillHooks.skills}
      activeSkillName={navProps.activeSkillName}
      importStatus={skillHooks.importStatus}
      importError={skillHooks.importError}
      onSelectSkill={navProps.setActiveSkillName}
      onImportClick={() => navProps.setImportDialogOpen(true)}
      onClearImportStatus={skillHooks.clearImportStatus}
    />
  );
}
