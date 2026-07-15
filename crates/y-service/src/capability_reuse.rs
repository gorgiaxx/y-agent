//! Bounded cross-asset reuse recommendations for self-orchestration.

use std::collections::HashSet;

use y_agent::TrustTier;

use crate::container::ServiceContainer;
use crate::workflow_service::WorkflowService;

const MIN_REUSE_SCORE: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAssetType {
    Skill,
    Agent,
    Tool,
    Workflow,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapabilityReuseRecommendation {
    pub asset_type: CapabilityAssetType,
    pub id: String,
    pub name: String,
    pub score: usize,
    pub reason: String,
    pub usage: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CapabilityReuseDecision {
    pub reuse_before_create: bool,
    pub recommendations: Vec<CapabilityReuseRecommendation>,
}

impl CapabilityReuseDecision {
    pub fn prompt_section(&self) -> Option<String> {
        if self.recommendations.is_empty() {
            return None;
        }
        let mut lines = vec![
            "<capability_reuse>".to_string(),
            if self.reuse_before_create {
                "Reuse these existing assets before creating a new skill, agent, tool, or workflow. Create only after verifying that every strong match is insufficient, and state the missing capability.".to_string()
            } else {
                "Consider these existing capabilities before designing new assets.".to_string()
            },
        ];
        for recommendation in &self.recommendations {
            lines.push(format!(
                "- {:?} `{}` (score {}): {} Usage: {}",
                recommendation.asset_type,
                recommendation.name,
                recommendation.score,
                recommendation.reason,
                recommendation.usage,
            ));
        }
        lines.push("</capability_reuse>".to_string());
        Some(lines.join("\n"))
    }
}

pub struct CapabilityReusePlanner;

impl CapabilityReusePlanner {
    pub async fn recommend(
        container: &ServiceContainer,
        query: &str,
        selected_skills: &[String],
    ) -> CapabilityReuseDecision {
        let mut candidates = Vec::new();
        let selected: HashSet<&str> = selected_skills.iter().map(String::as_str).collect();

        for matched in container
            .skill_search
            .read()
            .await
            .search_scored(query, 4, MIN_REUSE_SCORE)
        {
            let selected_note = if selected.contains(matched.summary.name.as_str()) {
                "already selected for this turn"
            } else {
                "strong skill match"
            };
            candidates.push(CapabilityReuseRecommendation {
                asset_type: CapabilityAssetType::Skill,
                id: matched.summary.id.to_string(),
                name: matched.summary.name,
                score: matched.score,
                reason: selected_note.to_string(),
                usage: "Use the selected skill instructions in this turn.".to_string(),
            });
        }

        {
            let registry = container.agent_registry.lock().await;
            for agent in registry.list() {
                if !agent.user_callable && agent.trust_tier != TrustTier::Dynamic {
                    continue;
                }
                let score =
                    text_match_score(&agent.name, &agent.description, &agent.capabilities, query);
                if score >= MIN_REUSE_SCORE {
                    candidates.push(CapabilityReuseRecommendation {
                        asset_type: CapabilityAssetType::Agent,
                        id: agent.id.clone(),
                        name: agent.name.clone(),
                        score,
                        reason: "existing callable agent matches the task".to_string(),
                        usage: format!("Delegate with Task using agent_name '{}'.", agent.id),
                    });
                }
            }
        }

        for tool in container.tool_registry.search_tools(query, None).await {
            let category = format!("{:?}", tool.category);
            let score = text_match_score(
                &tool.name.to_string(),
                &tool.description,
                &[category],
                query,
            );
            if score >= MIN_REUSE_SCORE {
                candidates.push(CapabilityReuseRecommendation {
                    asset_type: CapabilityAssetType::Tool,
                    id: tool.name.to_string(),
                    name: tool.name.to_string(),
                    score,
                    reason: "existing deterministic tool matches the requested action".to_string(),
                    usage: "Call the tool directly or resolve its schema with ToolSearch."
                        .to_string(),
                });
            }
        }

        if let Ok(workflows) = WorkflowService::list(&container.workflow_store).await {
            for workflow in workflows {
                let tags: Vec<String> = serde_json::from_str(&workflow.tags).unwrap_or_default();
                let score = text_match_score(
                    &workflow.name,
                    workflow.description.as_deref().unwrap_or_default(),
                    &tags,
                    query,
                );
                if score >= MIN_REUSE_SCORE {
                    candidates.push(CapabilityReuseRecommendation {
                        asset_type: CapabilityAssetType::Workflow,
                        id: workflow.id,
                        name: workflow.name,
                        score,
                        reason: "existing durable workflow matches the requested sequence"
                            .to_string(),
                        usage: "Run with WorkflowRun using this workflow id or name.".to_string(),
                    });
                }
            }
        }

        candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| asset_order(left.asset_type).cmp(&asset_order(right.asset_type)))
                .then_with(|| left.name.cmp(&right.name))
        });
        let mut seen_types = HashSet::new();
        let recommendations: Vec<_> = candidates
            .into_iter()
            .filter(|candidate| seen_types.insert(candidate.asset_type))
            .take(4)
            .collect();
        let reuse_before_create = recommendations.iter().any(|recommendation| {
            recommendation.score >= MIN_REUSE_SCORE
                && recommendation.asset_type != CapabilityAssetType::Tool
        });
        CapabilityReuseDecision {
            reuse_before_create,
            recommendations,
        }
    }
}

fn asset_order(asset_type: CapabilityAssetType) -> u8 {
    match asset_type {
        CapabilityAssetType::Skill => 0,
        CapabilityAssetType::Agent => 1,
        CapabilityAssetType::Tool => 2,
        CapabilityAssetType::Workflow => 3,
    }
}

fn text_match_score(name: &str, description: &str, tags: &[String], query: &str) -> usize {
    let name = normalize(name);
    let description = normalize(description);
    let tags: Vec<_> = tags.iter().map(|tag| normalize(tag)).collect();
    query_tokens(query)
        .into_iter()
        .map(|token| {
            let tag_score: usize = tags
                .iter()
                .map(|tag| {
                    if tag == &token {
                        12
                    } else if tag.contains(&token) || token.contains(tag) {
                        8
                    } else {
                        0
                    }
                })
                .sum();
            tag_score
                + usize::from(name.contains(&token)) * 4
                + usize::from(description.contains(&token)) * 2
        })
        .sum()
}

fn query_tokens(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "and", "are", "for", "from", "into", "please", "that", "the", "these", "this", "with",
    ];
    let mut tokens: Vec<_> = query
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.len() >= 3 && !STOP_WORDS.contains(&token.as_str()))
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn normalize(value: &str) -> String {
    value.to_lowercase().replace(['-', '_'], " ")
}
