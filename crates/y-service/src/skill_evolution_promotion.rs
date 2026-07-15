//! Validation, promotion, and rollback for approved skill proposals.

use std::collections::HashSet;

use y_core::skill::{SkillManifest, SkillVersion};
use y_skills::evolution::{EvolutionProposal, ProposalStatus, SkillMetrics};
use y_skills::experience::{ExperienceOutcome, ExperienceRecord};
use y_skills::regression::{RegressionDetector, RegressionResult};
use y_skills::{
    FilesystemSkillStore, PersistentVersionStore, SkillConfig, SkillModuleError, SkillValidator,
};

use super::{PromotionResources, SkillEvolutionService};

impl SkillEvolutionService {
    /// Validate and activate candidate root content for an approved proposal.
    pub async fn promote_approved_proposal(
        &self,
        proposal_id: &str,
        candidate_root_content: &str,
        resources: PromotionResources,
    ) -> Result<EvolutionProposal, SkillModuleError> {
        let _guard = self.cycle_lock.lock().await;
        if candidate_root_content.trim().is_empty() {
            return Err(SkillModuleError::Other {
                message: "candidate skill root content is empty".to_string(),
            });
        }

        let mut proposal = self.find_proposal(proposal_id).await?;
        if proposal.status != ProposalStatus::Approved {
            return Err(SkillModuleError::Other {
                message: format!("evolution proposal is not approved: {proposal_id}"),
            });
        }
        let skills_dir = self
            .skills_dir
            .as_ref()
            .ok_or_else(|| SkillModuleError::Other {
                message: "skill store is not configured".to_string(),
            })?;
        let skill_store = FilesystemSkillStore::new(skills_dir)?;
        let current = skill_store.load_skill(&proposal.skill_name)?;
        if current.version.0 != proposal.current_version {
            return Err(SkillModuleError::Other {
                message: format!(
                    "stale evolution proposal for '{}': expected version {}, found {}",
                    proposal.skill_name, proposal.current_version, current.version.0
                ),
            });
        }

        let mut candidate = current.clone();
        candidate.root_content = candidate_root_content.to_string();
        candidate.token_estimate = y_skills::manifest::estimate_tokens(candidate_root_content);
        candidate.updated_at = y_core::types::now();
        candidate.version = SkillVersion(String::new());
        Self::validate_candidate(&candidate, &skill_store, &resources)?;

        let baseline_bytes = Self::snapshot_bytes(&current)?;
        let candidate_bytes = Self::snapshot_bytes(&candidate)?;
        if baseline_bytes == candidate_bytes {
            return Err(SkillModuleError::Other {
                message: "candidate skill content does not change the active version".to_string(),
            });
        }

        let mut version_store = PersistentVersionStore::new(&self.version_store_path)?;
        let baseline_version =
            version_store.register_version(&proposal.skill_name, &baseline_bytes)?;
        let candidate_version =
            version_store.register_version(&proposal.skill_name, &candidate_bytes)?;
        candidate.version = candidate_version.clone();
        if let Err(error) = skill_store.save_skill(&candidate) {
            let _ = version_store.rollback(&proposal.skill_name, &baseline_version);
            return Err(error);
        }

        proposal.baseline_version = Some(baseline_version.0);
        proposal.proposed_version = Some(candidate_version.0);
        proposal.status = ProposalStatus::Promoted;
        self.proposal_journal.append(&proposal).await?;
        Ok(proposal)
    }

    /// Restore the parent version recorded by a promoted proposal.
    pub async fn rollback_promoted_proposal(
        &self,
        proposal_id: &str,
    ) -> Result<EvolutionProposal, SkillModuleError> {
        let _guard = self.cycle_lock.lock().await;
        let mut proposal = self.find_proposal(proposal_id).await?;
        if proposal.status != ProposalStatus::Promoted {
            return Err(SkillModuleError::Other {
                message: format!("evolution proposal is not promoted: {proposal_id}"),
            });
        }
        let skills_dir = self
            .skills_dir
            .as_ref()
            .ok_or_else(|| SkillModuleError::Other {
                message: "skill store is not configured".to_string(),
            })?;
        let target_version = SkillVersion(proposal.baseline_version.clone().ok_or_else(|| {
            SkillModuleError::Other {
                message: format!("promoted proposal has no baseline version: {proposal_id}"),
            }
        })?);
        let promoted_version =
            proposal
                .proposed_version
                .clone()
                .ok_or_else(|| SkillModuleError::Other {
                    message: format!("promoted proposal has no candidate version: {proposal_id}"),
                })?;
        let mut version_store = PersistentVersionStore::new(&self.version_store_path)?;
        version_store.rollback(&proposal.skill_name, &target_version)?;
        let snapshot = version_store.get(&target_version.0).ok_or_else(|| {
            SkillModuleError::VersionStoreError {
                message: format!("version {} not found in store", target_version.0),
            }
        })?;
        let mut manifest: SkillManifest = serde_json::from_slice(&snapshot)?;
        manifest.version = target_version.clone();
        manifest.updated_at = y_core::types::now();
        let skill_store = FilesystemSkillStore::new(skills_dir)?;
        if let Err(error) = skill_store.save_skill(&manifest) {
            let _ = version_store.rollback(&proposal.skill_name, &SkillVersion(promoted_version));
            return Err(error);
        }

        proposal.status = ProposalStatus::RolledBack;
        self.proposal_journal.append(&proposal).await?;
        Ok(proposal)
    }

    /// Compare post-promotion metrics with the parent version and roll back regressions.
    pub async fn evaluate_promoted_proposal(
        &self,
        proposal_id: &str,
    ) -> Result<RegressionResult, SkillModuleError> {
        let proposal = self.find_proposal(proposal_id).await?;
        if proposal.status != ProposalStatus::Promoted {
            return Err(SkillModuleError::Other {
                message: format!("evolution proposal is not promoted: {proposal_id}"),
            });
        }
        let candidate_version =
            proposal
                .proposed_version
                .as_deref()
                .ok_or_else(|| SkillModuleError::Other {
                    message: format!("promoted proposal has no candidate version: {proposal_id}"),
                })?;
        let experiences = self.experience_journal.load_all().await?;
        let baseline = Self::metrics_for_version(
            &experiences,
            &proposal.skill_name,
            &proposal.current_version,
        );
        let current =
            Self::metrics_for_version(&experiences, &proposal.skill_name, candidate_version);
        let regression = RegressionDetector::new().check(&baseline, &current);
        if regression.is_regression() {
            self.rollback_promoted_proposal(proposal_id).await?;
        }
        Ok(regression)
    }

    pub(super) async fn evaluate_active_promotions(&self) -> Result<(), SkillModuleError> {
        let promoted_ids: Vec<String> = self
            .proposal_journal
            .load_latest()
            .await?
            .into_iter()
            .filter(|proposal| proposal.status == ProposalStatus::Promoted)
            .map(|proposal| proposal.id)
            .collect();
        for proposal_id in promoted_ids {
            let _ = self.evaluate_promoted_proposal(&proposal_id).await?;
        }
        Ok(())
    }

    async fn find_proposal(
        &self,
        proposal_id: &str,
    ) -> Result<EvolutionProposal, SkillModuleError> {
        self.proposal_journal
            .load_latest()
            .await?
            .into_iter()
            .find(|proposal| proposal.id == proposal_id)
            .ok_or_else(|| SkillModuleError::Other {
                message: format!("evolution proposal not found: {proposal_id}"),
            })
    }

    pub(crate) fn validate_candidate(
        candidate: &SkillManifest,
        skill_store: &FilesystemSkillStore,
        resources: &PromotionResources,
    ) -> Result<(), SkillModuleError> {
        let validator = SkillValidator::new(SkillConfig::default());
        let registered_skills: HashSet<String> = skill_store
            .load_all()?
            .into_iter()
            .map(|manifest| manifest.name)
            .collect();
        let mut validation_copy = candidate.clone();
        // Promotion changes only root content, so existing approved security
        // capabilities are not a new privilege escalation.
        validation_copy.security = None;
        let mut errors = validator.validate_manifest(
            &validation_copy,
            &HashSet::new(),
            &resources.registered_tools,
            &registered_skills,
            &resources.registered_knowledge,
        );

        let temp = tempfile::tempdir().map_err(|error| SkillModuleError::Other {
            message: format!("failed to create candidate validation directory: {error}"),
        })?;
        let temp_store = FilesystemSkillStore::new(temp.path())?;
        temp_store.save_skill(candidate)?;
        errors.extend(validator.validate_directory(&temp.path().join(&candidate.name)));
        if errors.is_empty() {
            return Ok(());
        }
        Err(SkillModuleError::Other {
            message: format!(
                "candidate skill validation failed: {}",
                errors
                    .into_iter()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        })
    }

    fn snapshot_bytes(manifest: &SkillManifest) -> Result<Vec<u8>, SkillModuleError> {
        let mut snapshot = manifest.clone();
        snapshot.version = SkillVersion(String::new());
        serde_json::to_vec(&snapshot).map_err(SkillModuleError::from)
    }

    fn metrics_for_version(
        experiences: &[ExperienceRecord],
        skill_name: &str,
        version: &str,
    ) -> SkillMetrics {
        let mut metrics = SkillMetrics::default();
        for experience in experiences.iter().filter(|experience| {
            experience.skill_id.as_deref() == Some(skill_name)
                && experience.skill_version.as_deref() == Some(version)
        }) {
            metrics.record(
                experience.outcome == ExperienceOutcome::Success,
                experience.outcome == ExperienceOutcome::Partial,
                experience.duration_ms,
                u64::from(experience.token_usage.total),
            );
        }
        metrics
    }
}
