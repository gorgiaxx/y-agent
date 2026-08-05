//! Typed handoff artifacts for DAG nodes.
//!
//! Architecture reference: `docs/guides/ARCHITECTURE.md`
//!
//! A node's output is normally an opaque JSON blob: whether the work behind it
//! was thorough or perfunctory is invisible to the scheduler and to every
//! downstream node. The typed artifact makes thin work *structurally* visible
//! instead of relying on prompt wording.
//!
//! The load-bearing field is [`TaskArtifact::what_i_did_not_check`]. Forcing a
//! worker to enumerate what it did **not** cover is the one requirement that
//! cannot be satisfied by confident prose, which is why rigorous execution
//! rejects an artifact without it.

use serde::{Deserialize, Serialize};

/// Reserved input key under which a node receives its predecessors' artifacts.
pub const UPSTREAM_ARTIFACTS_INPUT: &str = "upstream_artifacts";

/// Key under which a node may nest its artifact inside its JSON output.
const ARTIFACT_OUTPUT_KEY: &str = "artifact";

/// The semantic role a node plays in the graph.
///
/// This is orthogonal to `TaskType`, which describes the *mechanism* (LLM call,
/// tool, sub-agent). A single mechanism serves several roles: a sub-agent call
/// may be exploration or verification, and the two carry different evidentiary
/// obligations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// Role not declared. Legacy and mechanically-generated nodes.
    #[default]
    Unspecified,
    /// Gathers information about a problem without changing anything.
    Explore,
    /// Performs the substantive change.
    Implement,
    /// Checks that an implementation satisfies its requirement.
    Verify,
    /// Repairs a defect surfaced by verification.
    Fix,
    /// Merges the artifacts of several nodes into one result.
    Synthesize,
    /// Challenges a result, looking for what earlier nodes missed.
    Critique,
}

impl NodeKind {
    /// Whether the role asserts coverage of a problem space and must therefore
    /// enumerate findings.
    ///
    /// `Implement` and `Fix` are excluded: their deliverable is the change
    /// itself, and demanding a findings list would produce filler.
    pub fn asserts_coverage(self) -> bool {
        matches!(
            self,
            NodeKind::Explore | NodeKind::Verify | NodeKind::Synthesize | NodeKind::Critique
        )
    }

    /// Whether the role's conclusion is only meaningful with evidence.
    pub fn requires_evidence(self) -> bool {
        matches!(self, NodeKind::Verify | NodeKind::Critique)
    }
}

/// How strictly artifacts are enforced.
///
/// One engine, one knob. `Deep` does not use a different scheduler or a
/// different dataflow; it only engages the artifact requirements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rigor {
    /// Artifacts are optional and unvalidated. Existing workflow behavior.
    #[default]
    Light,
    /// Artifacts are mandatory and validated against the node's role.
    Deep,
}

/// Machine-readable confidence rung parsed from the free-text confidence field.
///
/// The wire format stays a string for compatibility; this enum is the single
/// lenient interpretation of it, so no two call sites can disagree about what
/// "medium-ish" means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfidenceLevel {
    /// Scope was not adequately covered. Treated like an unaddressed gap.
    Low,
    /// Partial coverage with known holes.
    Medium,
    /// Scope believed fully covered.
    High,
}

impl ConfidenceLevel {
    /// Interpret a free-text confidence field.
    ///
    /// Unrecognized text yields [`ConfidenceLevel::Low`]: an unreadable
    /// confidence claim is an absent one, and absence must not read as strength.
    pub fn parse(raw: &str) -> ConfidenceLevel {
        let text = raw.trim().to_ascii_lowercase();

        if let Some(score) = leading_number(&text) {
            // Values above 1 are read as percentages, so "90" and "0.9" agree.
            let ratio = if score > 1.0 { score / 100.0 } else { score };
            return if ratio >= 0.8 {
                ConfidenceLevel::High
            } else if ratio >= 0.5 {
                ConfidenceLevel::Medium
            } else {
                ConfidenceLevel::Low
            };
        }

        if text.contains("high") || text.contains("certain") {
            ConfidenceLevel::High
        } else if text.contains("medium") || text.contains("moderate") {
            ConfidenceLevel::Medium
        } else {
            ConfidenceLevel::Low
        }
    }
}

/// Parse a number at the start of `text`, if any.
fn leading_number(text: &str) -> Option<f64> {
    let digits: String = text
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    digits.parse().ok()
}

/// The typed handoff payload a node attaches on completion.
///
/// It travels forward along edges to dependents, which receive it under
/// [`UPSTREAM_ARTIFACTS_INPUT`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskArtifact {
    /// One-paragraph result of the node.
    #[serde(default)]
    pub summary: String,
    /// What the node established.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    /// Concrete support for the findings: paths, commands, outputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    /// Edge cases the node deliberately considered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_cases_considered: Vec<String>,
    /// Scope the node knowingly left uncovered. The point of the whole type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub what_i_did_not_check: Vec<String>,
    /// Free-text confidence, interpreted by [`ConfidenceLevel::parse`].
    #[serde(default)]
    pub confidence: String,
}

impl TaskArtifact {
    /// Extract an artifact from a node's JSON output.
    ///
    /// Accepts either a nested `artifact` object or an output that is itself
    /// artifact-shaped, so executors are free to wrap or not.
    pub fn from_output(output: &serde_json::Value) -> Option<TaskArtifact> {
        let candidate = output.get(ARTIFACT_OUTPUT_KEY).unwrap_or(output);
        if !candidate.is_object() {
            return None;
        }
        let artifact: TaskArtifact = serde_json::from_value(candidate.clone()).ok()?;
        (artifact != TaskArtifact::default()).then_some(artifact)
    }

    /// Interpreted confidence rung.
    pub fn confidence_level(&self) -> ConfidenceLevel {
        ConfidenceLevel::parse(&self.confidence)
    }

    /// Validate the artifact against the node's role.
    ///
    /// Only called under [`Rigor::Deep`]; see [`validate_completion`].
    fn validate_for(&self, kind: NodeKind) -> Result<(), ArtifactError> {
        if self.summary.trim().is_empty() {
            return Err(ArtifactError::MissingField { field: "summary" });
        }
        if self.what_i_did_not_check.is_empty() {
            return Err(ArtifactError::MissingField {
                field: "what_i_did_not_check",
            });
        }
        if kind.asserts_coverage() && self.findings.is_empty() {
            return Err(ArtifactError::MissingField { field: "findings" });
        }
        if kind.requires_evidence() && self.evidence.is_empty() {
            return Err(ArtifactError::MissingField { field: "evidence" });
        }
        Ok(())
    }
}

/// Validate a completed node's output for the configured rigor.
///
/// Under [`Rigor::Light`] every output is accepted, which keeps existing
/// workflows working unchanged. Under [`Rigor::Deep`] the node must declare its
/// role and attach a complete artifact.
pub fn validate_completion(
    kind: NodeKind,
    rigor: Rigor,
    output: &serde_json::Value,
) -> Result<Option<TaskArtifact>, ArtifactError> {
    let artifact = TaskArtifact::from_output(output);
    if rigor == Rigor::Light {
        return Ok(artifact);
    }
    if kind == NodeKind::Unspecified {
        return Err(ArtifactError::UnspecifiedKind);
    }
    let artifact = artifact.ok_or(ArtifactError::Missing)?;
    artifact.validate_for(kind)?;
    Ok(Some(artifact))
}

/// Why an artifact was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactError {
    /// Deep rigor requires every node to declare its semantic role.
    #[error("node kind is unspecified; deep rigor requires a declared node kind")]
    UnspecifiedKind,

    /// The node produced no artifact at all.
    #[error("node produced no handoff artifact")]
    Missing,

    /// A required field was empty.
    #[error("handoff artifact is missing required field `{field}`")]
    MissingField {
        /// Name of the empty field.
        field: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_artifact() -> TaskArtifact {
        TaskArtifact {
            summary: "checked the retry path".into(),
            findings: vec!["retry is not applied to 429".into()],
            evidence: vec!["crates/y-provider/src/pool.rs:444".into()],
            edge_cases_considered: vec!["frozen provider".into()],
            what_i_did_not_check: vec!["streaming path".into()],
            confidence: "high".into(),
        }
    }

    #[test]
    fn confidence_words_map_to_rungs() {
        assert_eq!(ConfidenceLevel::parse("high"), ConfidenceLevel::High);
        assert_eq!(ConfidenceLevel::parse("Certain"), ConfidenceLevel::High);
        assert_eq!(ConfidenceLevel::parse("medium"), ConfidenceLevel::Medium);
        assert_eq!(ConfidenceLevel::parse("moderate"), ConfidenceLevel::Medium);
        assert_eq!(ConfidenceLevel::parse("low"), ConfidenceLevel::Low);
    }

    #[test]
    fn confidence_numbers_are_read_as_ratios_or_percentages() {
        assert_eq!(ConfidenceLevel::parse("0.9"), ConfidenceLevel::High);
        assert_eq!(ConfidenceLevel::parse("90"), ConfidenceLevel::High);
        assert_eq!(ConfidenceLevel::parse("0.6"), ConfidenceLevel::Medium);
        assert_eq!(ConfidenceLevel::parse("60"), ConfidenceLevel::Medium);
        assert_eq!(ConfidenceLevel::parse("0.2"), ConfidenceLevel::Low);
        assert_eq!(ConfidenceLevel::parse("1"), ConfidenceLevel::High);
    }

    #[test]
    fn unreadable_confidence_is_low_not_high() {
        assert_eq!(ConfidenceLevel::parse(""), ConfidenceLevel::Low);
        assert_eq!(ConfidenceLevel::parse("vibes"), ConfidenceLevel::Low);
    }

    #[test]
    fn confidence_rungs_are_ordered() {
        assert!(ConfidenceLevel::Low < ConfidenceLevel::Medium);
        assert!(ConfidenceLevel::Medium < ConfidenceLevel::High);
    }

    #[test]
    fn artifact_parses_from_nested_key_or_bare_object() {
        let nested = serde_json::json!({ "artifact": { "summary": "s" } });
        assert_eq!(
            TaskArtifact::from_output(&nested).map(|a| a.summary),
            Some("s".to_string())
        );

        let bare = serde_json::json!({ "summary": "s" });
        assert_eq!(
            TaskArtifact::from_output(&bare).map(|a| a.summary),
            Some("s".to_string())
        );
    }

    #[test]
    fn unrelated_output_yields_no_artifact() {
        let output = serde_json::json!({ "step": 1, "status": "completed" });
        assert_eq!(TaskArtifact::from_output(&output), None);
        assert_eq!(TaskArtifact::from_output(&serde_json::json!(42)), None);
    }

    #[test]
    fn light_rigor_accepts_any_output() {
        let output = serde_json::json!({ "step": 1 });
        let result = validate_completion(NodeKind::Unspecified, Rigor::Light, &output);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn light_rigor_still_carries_a_present_artifact() {
        let output = serde_json::json!({ "artifact": complete_artifact() });
        let carried = validate_completion(NodeKind::Explore, Rigor::Light, &output)
            .expect("light rigor accepts");
        assert_eq!(carried, Some(complete_artifact()));
    }

    #[test]
    fn deep_rigor_rejects_undeclared_node_kind() {
        let output = serde_json::json!({ "artifact": complete_artifact() });
        assert_eq!(
            validate_completion(NodeKind::Unspecified, Rigor::Deep, &output),
            Err(ArtifactError::UnspecifiedKind)
        );
    }

    #[test]
    fn deep_rigor_rejects_missing_artifact() {
        let output = serde_json::json!({ "step": 1 });
        assert_eq!(
            validate_completion(NodeKind::Implement, Rigor::Deep, &output),
            Err(ArtifactError::Missing)
        );
    }

    #[test]
    fn deep_rigor_requires_non_coverage_to_be_enumerated() {
        let mut artifact = complete_artifact();
        artifact.what_i_did_not_check.clear();
        let output = serde_json::json!({ "artifact": artifact });
        assert_eq!(
            validate_completion(NodeKind::Implement, Rigor::Deep, &output),
            Err(ArtifactError::MissingField {
                field: "what_i_did_not_check"
            })
        );
    }

    #[test]
    fn deep_rigor_requires_findings_only_from_coverage_roles() {
        let mut artifact = complete_artifact();
        artifact.findings.clear();
        let output = serde_json::json!({ "artifact": artifact });

        assert_eq!(
            validate_completion(NodeKind::Explore, Rigor::Deep, &output),
            Err(ArtifactError::MissingField { field: "findings" })
        );
        // An implementation's deliverable is the change, not a findings list.
        assert!(validate_completion(NodeKind::Implement, Rigor::Deep, &output).is_ok());
    }

    #[test]
    fn deep_rigor_requires_evidence_from_checking_roles() {
        let mut artifact = complete_artifact();
        artifact.evidence.clear();
        let output = serde_json::json!({ "artifact": artifact });

        for kind in [NodeKind::Verify, NodeKind::Critique] {
            assert_eq!(
                validate_completion(kind, Rigor::Deep, &output),
                Err(ArtifactError::MissingField { field: "evidence" }),
                "{kind:?} must show its work"
            );
        }
        assert!(validate_completion(NodeKind::Explore, Rigor::Deep, &output).is_ok());
    }

    #[test]
    fn deep_rigor_accepts_a_complete_artifact() {
        let output = serde_json::json!({ "artifact": complete_artifact() });
        for kind in [
            NodeKind::Explore,
            NodeKind::Implement,
            NodeKind::Verify,
            NodeKind::Fix,
            NodeKind::Synthesize,
            NodeKind::Critique,
        ] {
            assert!(
                validate_completion(kind, Rigor::Deep, &output).is_ok(),
                "{kind:?} rejected a complete artifact"
            );
        }
    }

    #[test]
    fn empty_summary_is_rejected_even_when_whitespace() {
        let mut artifact = complete_artifact();
        artifact.summary = "   ".into();
        let output = serde_json::json!({ "artifact": artifact });
        assert_eq!(
            validate_completion(NodeKind::Implement, Rigor::Deep, &output),
            Err(ArtifactError::MissingField { field: "summary" })
        );
    }

    #[test]
    fn node_kind_defaults_to_unspecified_and_round_trips() {
        assert_eq!(NodeKind::default(), NodeKind::Unspecified);
        let encoded = serde_json::to_string(&NodeKind::Synthesize).expect("serialize");
        assert_eq!(encoded, "\"synthesize\"");
        assert_eq!(
            serde_json::from_str::<NodeKind>(&encoded).expect("deserialize"),
            NodeKind::Synthesize
        );
    }

    #[test]
    fn rigor_defaults_to_light() {
        assert_eq!(Rigor::default(), Rigor::Light);
    }
}
