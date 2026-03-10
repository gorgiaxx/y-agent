//! `PromptTemplate`: declarative composition of sections with mode overlays.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::section::{SectionCondition, SectionId, TemplateId};

/// Reference to a section within a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionRef {
    /// Section ID to include.
    pub section_id: SectionId,
    /// Override the section's default priority.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_override: Option<i32>,
    /// Override the section's default condition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_override: Option<SectionCondition>,
    /// Set to false to exclude this section (useful in child templates).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Per-mode section adjustments.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModeOverlay {
    /// Additional sections to include in this mode.
    #[serde(default)]
    pub include: Vec<SectionId>,
    /// Sections to exclude in this mode.
    #[serde(default)]
    pub exclude: Vec<SectionId>,
    /// Per-section priority adjustments.
    #[serde(default)]
    pub priority_overrides: HashMap<SectionId, i32>,
    /// Override total token budget for this mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_budget_override: Option<u32>,
}

/// A prompt template — declarative composition of sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// Unique template identifier.
    pub id: TemplateId,
    /// Parent template for inheritance (single-parent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<TemplateId>,
    /// Ordered section references.
    pub sections: Vec<SectionRef>,
    /// Per-mode section adjustments.
    #[serde(default)]
    pub mode_overlays: HashMap<String, ModeOverlay>,
    /// Maximum total tokens for the assembled system prompt.
    #[serde(default = "default_total_budget")]
    pub total_token_budget: u32,
}

fn default_total_budget() -> u32 {
    4000
}

impl PromptTemplate {
    /// Get effective section refs after applying mode overlay.
    ///
    /// Returns the list of section IDs that are active for the given mode,
    /// with priority adjustments applied.
    pub fn effective_sections(&self, mode: &str) -> Vec<EffectiveSection> {
        let overlay = self.mode_overlays.get(mode);

        let mut result: Vec<EffectiveSection> = self
            .sections
            .iter()
            .filter(|s| s.enabled)
            .filter(|s| {
                // Exclude if the overlay says so.
                overlay
                    .is_none_or(|o| !o.exclude.contains(&s.section_id))
            })
            .map(|s| {
                let priority = overlay
                    .and_then(|o| o.priority_overrides.get(&s.section_id))
                    .copied()
                    .or(s.priority_override);

                EffectiveSection {
                    section_id: s.section_id.clone(),
                    priority_override: priority,
                    condition_override: s.condition_override.clone(),
                }
            })
            .collect();

        // Add overlay includes.
        if let Some(overlay) = overlay {
            for section_id in &overlay.include {
                if !result.iter().any(|s| s.section_id == *section_id) {
                    result.push(EffectiveSection {
                        section_id: section_id.clone(),
                        priority_override: overlay.priority_overrides.get(section_id).copied(),
                        condition_override: None,
                    });
                }
            }
        }

        result
    }

    /// Get the effective token budget for a mode.
    pub fn effective_budget(&self, mode: &str) -> u32 {
        self.mode_overlays
            .get(mode)
            .and_then(|o| o.token_budget_override)
            .unwrap_or(self.total_token_budget)
    }
}

/// A section reference after overlay resolution.
#[derive(Debug, Clone)]
pub struct EffectiveSection {
    pub section_id: SectionId,
    pub priority_override: Option<i32>,
    pub condition_override: Option<SectionCondition>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_template() -> PromptTemplate {
        PromptTemplate {
            id: "default".into(),
            parent: None,
            sections: vec![
                SectionRef {
                    section_id: "core.identity".into(),
                    priority_override: None,
                    condition_override: None,
                    enabled: true,
                },
                SectionRef {
                    section_id: "core.safety".into(),
                    priority_override: None,
                    condition_override: None,
                    enabled: true,
                },
                SectionRef {
                    section_id: "core.tool_behavior".into(),
                    priority_override: None,
                    condition_override: None,
                    enabled: true,
                },
            ],
            mode_overlays: {
                let mut m = HashMap::new();
                m.insert(
                    "plan".into(),
                    ModeOverlay {
                        exclude: vec!["core.tool_behavior".into()],
                        include: vec!["core.planning".into()],
                        ..Default::default()
                    },
                );
                m.insert(
                    "explore".into(),
                    ModeOverlay {
                        exclude: vec!["core.safety".into()],
                        include: vec!["core.exploration".into()],
                        token_budget_override: Some(2000),
                        ..Default::default()
                    },
                );
                m
            },
            total_token_budget: 4000,
        }
    }

    #[test]
    fn test_template_effective_sections_general_mode() {
        let t = base_template();
        let eff = t.effective_sections("general");
        assert_eq!(eff.len(), 3); // All 3 sections active, no overlay for "general".
    }

    #[test]
    fn test_template_mode_overlay_excludes() {
        let t = base_template();
        let eff = t.effective_sections("plan");
        let ids: Vec<&str> = eff.iter().map(|s| s.section_id.as_str()).collect();
        assert!(!ids.contains(&"core.tool_behavior"));
        assert!(ids.contains(&"core.planning"));
    }

    #[test]
    fn test_template_mode_overlay_includes() {
        let t = base_template();
        let eff = t.effective_sections("explore");
        let ids: Vec<&str> = eff.iter().map(|s| s.section_id.as_str()).collect();
        assert!(ids.contains(&"core.exploration"));
        assert!(!ids.contains(&"core.safety"));
    }

    #[test]
    fn test_template_effective_budget() {
        let t = base_template();
        assert_eq!(t.effective_budget("general"), 4000);
        assert_eq!(t.effective_budget("explore"), 2000);
    }

    #[test]
    fn test_template_serialization() {
        let t = base_template();
        let json = serde_json::to_string(&t).unwrap();
        let roundtrip: PromptTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.id, "default");
        assert_eq!(roundtrip.sections.len(), 3);
    }

    #[test]
    fn test_template_disabled_section_excluded() {
        let mut t = base_template();
        t.sections[1].enabled = false; // Disable core.safety.
        let eff = t.effective_sections("general");
        let ids: Vec<&str> = eff.iter().map(|s| s.section_id.as_str()).collect();
        assert!(!ids.contains(&"core.safety"));
        assert_eq!(eff.len(), 2);
    }
}
