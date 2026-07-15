//! Service-layer orchestration for governed skill evolution.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use tokio::sync::Mutex;
use y_core::skill::SkillManifest;
use y_skills::evolution::{ChangeType, EvolutionProposal, PatternType, ProposalStatus};
use y_skills::experience::{
    EvidenceEntry, EvidenceProvenance, ExperienceOutcome, ExperienceRecord, TokenUsage,
    ToolCallRecord,
};
use y_skills::{
    ExperienceJournal, FilesystemSkillStore, PatternExtractor, ProposalJournal, SkillModuleError,
};

/// Complete turn evidence supplied by the chat or orchestration service.
#[derive(Debug, Clone)]
pub struct TurnExperienceInput {
    /// Skills that influenced the turn. Empty means a skillless experience.
    pub skills: Vec<String>,
    /// User-visible task attempted by the turn.
    pub task_description: String,
    /// Objective execution outcome.
    pub outcome: ExperienceOutcome,
    /// Concise execution trajectory summary.
    pub trajectory_summary: String,
    /// Important decisions made during execution.
    pub key_decisions: Vec<String>,
    /// Provenance-tagged evidence supporting the outcome.
    pub evidence: Vec<EvidenceEntry>,
    /// Tool calls made during execution.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Errors observed during execution.
    pub error_messages: Vec<String>,
    /// End-to-end turn duration in milliseconds.
    pub duration_ms: u64,
    /// Prompt and completion token usage.
    pub token_usage: TokenUsage,
}

/// Registries used to validate cross-resource references during promotion.
#[derive(Debug, Clone, Default)]
pub struct PromotionResources {
    /// Registered tool names.
    pub registered_tools: HashSet<String>,
    /// Registered knowledge collection names.
    pub registered_knowledge: HashSet<String>,
}

/// Supervised decision accepted by the skill-evolution control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillProposalDecision {
    Approve,
    Reject,
    Defer,
}

/// Coordinates durable experience capture and pending proposal generation.
///
/// This service never mutates active skills. It only records evidence and
/// creates proposals with [`ProposalStatus::PendingApproval`].
#[derive(Debug)]
pub struct SkillEvolutionService {
    experience_journal: ExperienceJournal,
    proposal_journal: ProposalJournal,
    skills_dir: Option<PathBuf>,
    version_store_path: PathBuf,
    cycle_lock: Mutex<()>,
}

impl SkillEvolutionService {
    /// Open the evolution journals rooted at `data_dir`.
    pub async fn open(
        data_dir: impl AsRef<Path>,
        skills_dir: Option<PathBuf>,
    ) -> Result<Self, SkillModuleError> {
        let data_dir = data_dir.as_ref();
        Ok(Self {
            experience_journal: ExperienceJournal::open(data_dir.join("experiences.jsonl")).await?,
            proposal_journal: ProposalJournal::open(data_dir.join("proposals.jsonl")).await?,
            skills_dir,
            version_store_path: data_dir.join("versions"),
            cycle_lock: Mutex::new(()),
        })
    }

    /// Persist one turn and generate newly eligible pending proposals.
    pub async fn record_turn(
        &self,
        mut input: TurnExperienceInput,
    ) -> Result<Vec<EvolutionProposal>, SkillModuleError> {
        let created = {
            let _guard = self.cycle_lock.lock().await;
            input.evidence = Self::sanitize_evidence(input.evidence);
            Self::ensure_task_outcome_evidence(&mut input);

            let skills: BTreeSet<String> = input
                .skills
                .iter()
                .map(|skill| skill.trim())
                .filter(|skill| !skill.is_empty())
                .map(String::from)
                .collect();

            if skills.is_empty() {
                self.experience_journal
                    .append(&Self::build_record(None, None, &input))
                    .await?;
            } else {
                for skill in skills {
                    let version = self.resolve_skill_version(&skill);
                    self.experience_journal
                        .append(&Self::build_record(Some(skill), version, &input))
                        .await?;
                }
            }

            self.generate_pending_proposals().await?
        };
        self.evaluate_active_promotions().await?;
        Ok(created)
    }

    /// Persist explicit user feedback as idempotent, trace-linked evolution evidence.
    pub async fn record_user_feedback(
        &self,
        feedback_id: uuid::Uuid,
        trace_id: uuid::Uuid,
        skills: &[String],
        task_description: &str,
        score: f64,
        comment: &str,
    ) -> Result<usize, SkillModuleError> {
        if !score.is_finite() || !(0.0..=1.0).contains(&score) {
            return Err(SkillModuleError::Other {
                message: "user feedback score must be between 0.0 and 1.0".to_string(),
            });
        }
        if comment.trim().is_empty() {
            return Err(SkillModuleError::Other {
                message: "user feedback comment must not be blank".to_string(),
            });
        }

        let outcome = if score >= 0.75 {
            ExperienceOutcome::Success
        } else if score <= 0.25 {
            ExperienceOutcome::Failure
        } else {
            ExperienceOutcome::Partial
        };
        let provenance = if outcome == ExperienceOutcome::Failure {
            EvidenceProvenance::UserCorrection
        } else {
            EvidenceProvenance::UserStated
        };
        let mut input = TurnExperienceInput {
            skills: skills.to_vec(),
            task_description: task_description.to_string(),
            outcome,
            trajectory_summary: format!(
                "User rated diagnostics trace {trace_id} with score {score:.2}"
            ),
            key_decisions: vec![format!("Feedback applies to diagnostics trace {trace_id}")],
            evidence: vec![EvidenceEntry {
                content: comment.trim().to_string(),
                provenance,
            }],
            tool_calls: Vec::new(),
            error_messages: Vec::new(),
            duration_ms: 0,
            token_usage: TokenUsage::default(),
        };

        let appended = {
            let _guard = self.cycle_lock.lock().await;
            Self::ensure_task_outcome_evidence(&mut input);
            let existing: HashSet<String> = self
                .experience_journal
                .load_all()
                .await?
                .into_iter()
                .map(|record| record.id)
                .collect();
            let skill_targets: BTreeSet<String> = skills
                .iter()
                .map(|skill| skill.trim())
                .filter(|skill| !skill.is_empty())
                .map(String::from)
                .collect();
            let targets: Vec<Option<String>> = if skill_targets.is_empty() {
                vec![None]
            } else {
                skill_targets.into_iter().map(Some).collect()
            };
            let mut appended = 0;
            for skill in targets {
                let target = skill.as_deref().unwrap_or("none");
                let record_id = format!("feedback:{feedback_id}:{target}");
                if existing.contains(&record_id) {
                    continue;
                }
                let version = skill
                    .as_deref()
                    .and_then(|skill_name| self.resolve_skill_version(skill_name));
                let mut record = Self::build_record(skill, version, &input);
                record.id = record_id;
                self.experience_journal.append(&record).await?;
                appended += 1;
            }
            if appended > 0 {
                self.generate_pending_proposals().await?;
            }
            appended
        };
        self.evaluate_active_promotions().await?;
        Ok(appended)
    }

    /// Load all captured experiences in append order.
    pub async fn load_experiences(&self) -> Result<Vec<ExperienceRecord>, SkillModuleError> {
        self.experience_journal.load_all().await
    }

    /// Load the latest state of every evolution proposal.
    pub async fn load_proposals(&self) -> Result<Vec<EvolutionProposal>, SkillModuleError> {
        self.proposal_journal.load_latest().await
    }

    /// Load one proposal's latest durable state.
    pub async fn get_proposal(
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

    /// Load the active manifest targeted by a proposal.
    pub fn load_active_skill(&self, skill_name: &str) -> Result<SkillManifest, SkillModuleError> {
        let skills_dir = self
            .skills_dir
            .as_ref()
            .ok_or_else(|| SkillModuleError::Other {
                message: "skill store is not configured".to_string(),
            })?;
        FilesystemSkillStore::new(skills_dir)?.load_skill(skill_name)
    }

    /// Return recent evidence for one skill, newest records last.
    pub async fn recent_skill_experiences(
        &self,
        skill_name: &str,
        limit: usize,
    ) -> Result<Vec<ExperienceRecord>, SkillModuleError> {
        let mut records: Vec<_> = self
            .experience_journal
            .load_all()
            .await?
            .into_iter()
            .filter(|record| record.skill_id.as_deref() == Some(skill_name))
            .collect();
        let keep_from = records.len().saturating_sub(limit.max(1));
        records.drain(0..keep_from);
        Ok(records)
    }

    /// Validate and persist a refinement candidate without activating it.
    pub async fn attach_candidate(
        &self,
        proposal_id: &str,
        root_content: String,
        rationale: String,
        resources: &PromotionResources,
    ) -> Result<EvolutionProposal, SkillModuleError> {
        let _guard = self.cycle_lock.lock().await;
        if root_content.trim().is_empty() || rationale.trim().is_empty() {
            return Err(SkillModuleError::Other {
                message: "skill refinement requires non-empty root content and rationale"
                    .to_string(),
            });
        }
        let mut proposal = self.get_proposal(proposal_id).await?;
        if !matches!(
            proposal.status,
            ProposalStatus::PendingApproval | ProposalStatus::Deferred
        ) {
            return Err(SkillModuleError::Other {
                message: format!(
                    "evolution proposal cannot be refined in status {:?}: {proposal_id}",
                    proposal.status
                ),
            });
        }
        let current = self.load_active_skill(&proposal.skill_name)?;
        if current.version.0 != proposal.current_version {
            return Err(SkillModuleError::Other {
                message: format!(
                    "stale evolution proposal for '{}': expected version {}, found {}",
                    proposal.skill_name, proposal.current_version, current.version.0
                ),
            });
        }
        let mut candidate = current.clone();
        candidate.root_content = root_content.trim().to_string();
        candidate.token_estimate = y_skills::manifest::estimate_tokens(&candidate.root_content);
        candidate.updated_at = y_core::types::now();
        candidate.version = y_core::skill::SkillVersion(String::new());
        let skills_dir = self
            .skills_dir
            .as_ref()
            .ok_or_else(|| SkillModuleError::Other {
                message: "skill store is not configured".to_string(),
            })?;
        let skill_store = FilesystemSkillStore::new(skills_dir)?;
        Self::validate_candidate(&candidate, &skill_store, resources)?;
        if candidate.root_content == current.root_content {
            return Err(SkillModuleError::Other {
                message: "candidate skill content does not change the active version".to_string(),
            });
        }

        proposal.candidate_root_content = Some(candidate.root_content.clone());
        proposal.candidate_rationale = Some(rationale.trim().to_string());
        proposal.diff_preview =
            bounded_diff_preview(&current.root_content, &candidate.root_content);
        proposal.decision_reason = None;
        proposal.status = ProposalStatus::PendingApproval;
        self.proposal_journal.append(&proposal).await?;
        Ok(proposal)
    }

    /// Apply a supervised proposal decision.
    ///
    /// Approval activates only a previously validated persisted candidate.
    /// Rejection and deferral update journal state without touching the skill.
    pub async fn decide_proposal(
        &self,
        proposal_id: &str,
        decision: SkillProposalDecision,
        reason: Option<String>,
        resources: PromotionResources,
    ) -> Result<EvolutionProposal, SkillModuleError> {
        let reason = reason
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if decision == SkillProposalDecision::Approve {
            let candidate = {
                let _guard = self.cycle_lock.lock().await;
                let mut proposal = self.get_proposal(proposal_id).await?;
                if !matches!(
                    proposal.status,
                    ProposalStatus::PendingApproval | ProposalStatus::Deferred
                ) {
                    return Err(SkillModuleError::Other {
                        message: format!(
                            "evolution proposal cannot be approved in status {:?}: {proposal_id}",
                            proposal.status
                        ),
                    });
                }
                let candidate = proposal.candidate_root_content.clone().ok_or_else(|| {
                    SkillModuleError::Other {
                        message: format!(
                            "evolution proposal has no validated candidate: {proposal_id}"
                        ),
                    }
                })?;
                proposal.status = ProposalStatus::Approved;
                proposal.decision_reason = reason.clone();
                self.proposal_journal.append(&proposal).await?;
                candidate
            };

            return match self
                .promote_approved_proposal(proposal_id, &candidate, resources)
                .await
            {
                Ok(proposal) => Ok(proposal),
                Err(error) => {
                    let _guard = self.cycle_lock.lock().await;
                    let mut proposal = self.get_proposal(proposal_id).await?;
                    if proposal.status == ProposalStatus::Approved {
                        proposal.status = ProposalStatus::PendingApproval;
                        proposal.decision_reason = Some(format!(
                            "approval could not be applied and remains pending: {error}"
                        ));
                        self.proposal_journal.append(&proposal).await?;
                    }
                    Err(error)
                }
            };
        }

        let _guard = self.cycle_lock.lock().await;
        let mut proposal = self.get_proposal(proposal_id).await?;
        if matches!(
            proposal.status,
            ProposalStatus::Approved
                | ProposalStatus::Rejected
                | ProposalStatus::Promoted
                | ProposalStatus::RolledBack
        ) {
            return Err(SkillModuleError::Other {
                message: format!("evolution proposal is already terminal: {proposal_id}"),
            });
        }
        proposal.status = match decision {
            SkillProposalDecision::Reject => ProposalStatus::Rejected,
            SkillProposalDecision::Defer => ProposalStatus::Deferred,
            SkillProposalDecision::Approve => unreachable!(),
        };
        proposal.decision_reason = reason;
        self.proposal_journal.append(&proposal).await?;
        Ok(proposal)
    }

    /// Persist a supervised approval, rejection, or deferral decision.
    ///
    /// This changes proposal state only; it never edits or activates a skill.
    pub async fn update_proposal_status(
        &self,
        proposal_id: &str,
        status: ProposalStatus,
    ) -> Result<EvolutionProposal, SkillModuleError> {
        let _guard = self.cycle_lock.lock().await;
        if !matches!(
            status,
            ProposalStatus::Approved | ProposalStatus::Rejected | ProposalStatus::Deferred
        ) {
            return Err(SkillModuleError::Other {
                message: "proposal decisions must approve, reject, or defer".to_string(),
            });
        }
        let mut proposal = self
            .proposal_journal
            .load_latest()
            .await?
            .into_iter()
            .find(|proposal| proposal.id == proposal_id)
            .ok_or_else(|| SkillModuleError::Other {
                message: format!("evolution proposal not found: {proposal_id}"),
            })?;
        if matches!(
            proposal.status,
            ProposalStatus::Approved
                | ProposalStatus::Rejected
                | ProposalStatus::Promoted
                | ProposalStatus::RolledBack
        ) {
            return Err(SkillModuleError::Other {
                message: format!("evolution proposal is already terminal: {proposal_id}"),
            });
        }

        proposal.status = status;
        self.proposal_journal.append(&proposal).await?;
        Ok(proposal)
    }

    fn build_record(
        skill_id: Option<String>,
        skill_version: Option<String>,
        input: &TurnExperienceInput,
    ) -> ExperienceRecord {
        ExperienceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            skill_id,
            skill_version,
            task_description: input.task_description.clone(),
            outcome: input.outcome,
            trajectory_summary: input.trajectory_summary.clone(),
            key_decisions: input.key_decisions.clone(),
            evidence: input.evidence.clone(),
            tool_calls: input.tool_calls.clone(),
            error_messages: input.error_messages.clone(),
            duration_ms: input.duration_ms,
            token_usage: input.token_usage.clone(),
        }
    }

    fn resolve_skill_version(&self, skill_name: &str) -> Option<String> {
        let skills_dir = self.skills_dir.as_ref()?;
        let store = FilesystemSkillStore::new(skills_dir).ok()?;
        store
            .load_skill(skill_name)
            .ok()
            .map(|manifest| manifest.version.0)
            .filter(|version| !version.is_empty())
    }

    async fn generate_pending_proposals(&self) -> Result<Vec<EvolutionProposal>, SkillModuleError> {
        let experiences = self.experience_journal.load_all().await?;
        let patterns = PatternExtractor::new().extract(&experiences);
        let existing = self.proposal_journal.load_latest().await?;
        let referenced_patterns: HashSet<&str> = existing
            .iter()
            .flat_map(|proposal| proposal.patterns_referenced.iter().map(String::as_str))
            .collect();
        let experiences_by_id: HashMap<&str, &ExperienceRecord> = experiences
            .iter()
            .map(|experience| (experience.id.as_str(), experience))
            .collect();

        let mut by_skill = BTreeMap::<String, Vec<_>>::new();
        for pattern in patterns {
            if pattern.evidence_ids.len() >= 2
                && !referenced_patterns.contains(pattern.id.as_str())
                && Self::has_actionable_evidence(&pattern.evidence_ids, &experiences_by_id)
            {
                by_skill
                    .entry(pattern.skill_id.clone())
                    .or_default()
                    .push(pattern);
            }
        }

        let mut created = Vec::new();
        for (skill_id, skill_patterns) in by_skill {
            let current_version = experiences
                .iter()
                .rev()
                .find(|experience| experience.skill_id.as_deref() == Some(&skill_id))
                .and_then(|experience| experience.skill_version.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let pattern_types = skill_patterns
                .iter()
                .fold(Vec::new(), |mut types, pattern| {
                    if !types.contains(&pattern.pattern_type) {
                        types.push(pattern.pattern_type);
                    }
                    types
                });
            let change_type =
                (pattern_types.len() == 1).then(|| Self::change_type(pattern_types[0]));
            let proposed_changes = skill_patterns
                .iter()
                .map(|pattern| format!("{}: {}", pattern.pattern_type, pattern.description))
                .collect::<Vec<_>>()
                .join("; ");
            let proposal = EvolutionProposal {
                id: format!("proposal-{}", uuid::Uuid::new_v4()),
                skill_name: skill_id,
                current_version,
                proposed_changes: format!("Address observed patterns: {proposed_changes}"),
                patterns: pattern_types,
                status: ProposalStatus::PendingApproval,
                proposed_version: None,
                baseline_version: None,
                change_type,
                patterns_referenced: skill_patterns
                    .into_iter()
                    .map(|pattern| pattern.id)
                    .collect(),
                diff_preview: String::new(),
                candidate_root_content: None,
                candidate_rationale: None,
                decision_reason: None,
                deferred_until: None,
            };
            self.proposal_journal.append(&proposal).await?;
            created.push(proposal);
        }
        Ok(created)
    }

    fn has_actionable_evidence(
        evidence_ids: &[String],
        experiences: &HashMap<&str, &ExperienceRecord>,
    ) -> bool {
        let records: Vec<_> = evidence_ids
            .iter()
            .filter_map(|id| experiences.get(id.as_str()).copied())
            .collect();
        let has_user_evidence = records.iter().any(|record| {
            record.evidence.iter().any(|entry| {
                matches!(
                    entry.provenance,
                    EvidenceProvenance::UserStated | EvidenceProvenance::UserCorrection
                )
            })
        });
        let failed_tool_records = records
            .iter()
            .filter(|record| record.tool_calls.iter().any(|call| !call.success))
            .count();

        has_user_evidence || failed_tool_records >= 2
    }

    fn change_type(pattern_type: PatternType) -> ChangeType {
        match pattern_type {
            PatternType::EdgeCase => ChangeType::EdgeCaseAddition,
            PatternType::CommonError => ChangeType::ErrorWarning,
            PatternType::BetterPhrasing => ChangeType::PhrasingUpdate,
            PatternType::NewCapability => ChangeType::CapabilitySplit,
            PatternType::ObsoleteRule => ChangeType::RuleRemoval,
            PatternType::WorkflowDiscovery => ChangeType::WorkflowDiscovery,
        }
    }

    fn sanitize_evidence(evidence: Vec<EvidenceEntry>) -> Vec<EvidenceEntry> {
        let corroborated: HashSet<String> = evidence
            .iter()
            .filter(|entry| {
                matches!(
                    entry.provenance,
                    EvidenceProvenance::UserStated | EvidenceProvenance::UserCorrection
                )
            })
            .map(|entry| entry.content.trim().to_lowercase())
            .collect();

        evidence
            .into_iter()
            .filter(|entry| {
                entry.provenance != EvidenceProvenance::AgentObservation
                    || corroborated.contains(&entry.content.trim().to_lowercase())
            })
            .collect()
    }

    fn ensure_task_outcome_evidence(input: &mut TurnExperienceInput) {
        if input
            .evidence
            .iter()
            .any(|entry| entry.provenance == EvidenceProvenance::TaskOutcome)
        {
            return;
        }
        let outcome = match input.outcome {
            ExperienceOutcome::Success => "success",
            ExperienceOutcome::Partial => "partial",
            ExperienceOutcome::Failure => "failure",
        };
        input.evidence.push(EvidenceEntry {
            content: format!("Turn completed with outcome: {outcome}"),
            provenance: EvidenceProvenance::TaskOutcome,
        });
    }
}

fn bounded_diff_preview(current: &str, candidate: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 4_000;
    let preview = format!("--- active\n+++ candidate\n-{current}\n+{candidate}");
    if preview.chars().count() <= MAX_PREVIEW_CHARS {
        return preview;
    }
    preview.chars().take(MAX_PREVIEW_CHARS).collect()
}

#[path = "skill_evolution_promotion.rs"]
mod promotion;

#[cfg(test)]
#[path = "skill_evolution_service_tests.rs"]
mod tests;
