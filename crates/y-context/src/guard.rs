//! Context Window Guard: monitors token usage and triggers compaction.

use serde::{Deserialize, Serialize};

use crate::pipeline::{AssembledContext, ContextCategory};

/// Guard trigger mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GuardMode {
    /// System triggers compaction at threshold.
    Auto,
    /// Agent is expected to compress proactively; hard limit at 95%.
    Soft,
    /// Soft warnings + auto fallback at threshold.
    #[default]
    Hybrid,
}

/// Token budget for context categories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    pub system_prompt: u32,
    pub tools_schema: u32,
    pub history: u32,
    pub bootstrap: u32,
    pub response_reserve: u32,
}

impl TokenBudget {
    /// Total budget across all categories (excluding response reserve).
    pub fn total_available(&self) -> u32 {
        self.system_prompt
            .saturating_add(self.tools_schema)
            .saturating_add(self.history)
            .saturating_add(self.bootstrap)
    }

    /// Total budget including response reserve.
    pub fn total_with_reserve(&self) -> u32 {
        self.total_available().saturating_add(self.response_reserve)
    }
}

impl Default for TokenBudget {
    fn default() -> Self {
        Self {
            system_prompt: 8_000,
            tools_schema: 16_000,
            history: 80_000,
            bootstrap: 8_000,
            response_reserve: 16_000,
        }
    }
}

/// Guard verdict after evaluating context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardVerdict {
    /// Context is within budget.
    Ok,
    /// Warning: approaching threshold.
    Warning { utilization_pct: u32 },
    /// Overflow: compaction needed.
    Overflow { tokens_over: u32 },
    /// Critical: emergency compaction required.
    Critical { tokens_over: u32 },
}

/// Context Window Guard.
pub struct ContextWindowGuard {
    pub mode: GuardMode,
    pub budget: TokenBudget,
    /// Warning threshold (percentage, default 70).
    pub warning_threshold: u32,
    /// Compaction threshold (percentage, default 85).
    pub compaction_threshold: u32,
    /// Critical threshold (percentage, default 95).
    pub critical_threshold: u32,
}

impl ContextWindowGuard {
    /// Create a new guard with default settings.
    pub fn new() -> Self {
        Self {
            mode: GuardMode::default(),
            budget: TokenBudget::default(),
            warning_threshold: 70,
            compaction_threshold: 85,
            critical_threshold: 95,
        }
    }

    /// Evaluate assembled context against the budget.
    pub fn evaluate(&self, ctx: &AssembledContext) -> GuardVerdict {
        let total = self.budget.total_available();
        let used = ctx.total_tokens();
        let utilization = utilization_pct(used, total);

        if utilization >= self.critical_threshold {
            GuardVerdict::Critical {
                tokens_over: used.saturating_sub(total),
            }
        } else if utilization >= self.compaction_threshold {
            GuardVerdict::Overflow {
                tokens_over: used
                    .saturating_sub(threshold_tokens(total, self.compaction_threshold)),
            }
        } else if utilization >= self.warning_threshold {
            GuardVerdict::Warning {
                utilization_pct: utilization,
            }
        } else {
            GuardVerdict::Ok
        }
    }

    /// Generate context status message for injection.
    pub fn status_message(&self, ctx: &AssembledContext) -> String {
        let total = self.budget.total_available();
        let used = ctx.total_tokens();
        let utilization = utilization_pct(used, total);

        let base = format!(
            "[Context Status: working_tokens={used}, threshold={total}, utilization={utilization}%]"
        );

        if utilization >= 95 {
            format!("{base}\nCRITICAL: context overflow imminent. System compaction will be triggered if compress_experience is not called.")
        } else if utilization >= 85 {
            format!("{base}\nWARNING: working context approaching threshold. Use compress_experience now to avoid forced compaction.")
        } else if utilization >= 70 {
            format!("{base}\nConsider using compress_experience to archive evidence before context grows further.")
        } else {
            base
        }
    }

    /// Check if a specific category is over budget.
    pub fn category_over_budget(
        &self,
        ctx: &AssembledContext,
        category: ContextCategory,
    ) -> Option<u32> {
        let used = ctx.tokens_for(category);
        let budget = match category {
            ContextCategory::SystemPrompt => self.budget.system_prompt,
            ContextCategory::Bootstrap => self.budget.bootstrap,
            ContextCategory::Tools => self.budget.tools_schema,
            ContextCategory::History => self.budget.history,
            _ => return None, // Other categories don't have individual budgets.
        };
        if used > budget {
            Some(used - budget)
        } else {
            None
        }
    }
}

impl Default for ContextWindowGuard {
    fn default() -> Self {
        Self::new()
    }
}

fn utilization_pct(used: u32, total: u32) -> u32 {
    if total == 0 {
        return 100;
    }

    u32::try_from(u64::from(used) * 100 / u64::from(total)).unwrap_or(100)
}

fn threshold_tokens(total: u32, threshold_pct: u32) -> u32 {
    u32::try_from(u64::from(total) * u64::from(threshold_pct) / 100).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use crate::pipeline::ContextItem;

    use super::*;

    fn make_ctx(tokens: u32) -> AssembledContext {
        let mut ctx = AssembledContext::default();
        ctx.add(ContextItem {
            category: ContextCategory::History,
            content: String::new(),
            token_estimate: tokens,
            priority: 0,
        });
        ctx
    }

    #[test]
    fn test_guard_ok() {
        let guard = ContextWindowGuard::new();
        let ctx = make_ctx(50_000); // < 70% of 112K
        assert_eq!(guard.evaluate(&ctx), GuardVerdict::Ok);
    }

    #[test]
    fn test_guard_warning() {
        let guard = ContextWindowGuard::new();
        // 70% of 112K = 78,400
        let ctx = make_ctx(80_000);
        assert!(matches!(guard.evaluate(&ctx), GuardVerdict::Warning { .. }));
    }

    #[test]
    fn test_guard_overflow() {
        let guard = ContextWindowGuard::new();
        // 85% of 112K = 95,200
        let ctx = make_ctx(96_000);
        assert!(matches!(
            guard.evaluate(&ctx),
            GuardVerdict::Overflow { .. }
        ));
    }

    #[test]
    fn test_guard_critical() {
        let guard = ContextWindowGuard::new();
        // 95% of 112K = 106,400
        let ctx = make_ctx(110_000);
        assert!(matches!(
            guard.evaluate(&ctx),
            GuardVerdict::Critical { .. }
        ));
    }

    #[test]
    fn test_status_message_levels() {
        let guard = ContextWindowGuard::new();

        let msg = guard.status_message(&make_ctx(50_000));
        assert!(!msg.contains("WARNING"));

        let msg = guard.status_message(&make_ctx(80_000));
        assert!(msg.contains("compress_experience"));

        let msg = guard.status_message(&make_ctx(97_000));
        assert!(msg.contains("WARNING"));

        let msg = guard.status_message(&make_ctx(110_000));
        assert!(msg.contains("CRITICAL"));
    }

    #[test]
    fn test_category_over_budget() {
        let guard = ContextWindowGuard::new();
        let mut ctx = AssembledContext::default();
        ctx.add(ContextItem {
            category: ContextCategory::History,
            content: String::new(),
            token_estimate: 90_000, // Over 80K budget.
            priority: 0,
        });
        assert_eq!(
            guard.category_over_budget(&ctx, ContextCategory::History),
            Some(10_000)
        );
        assert_eq!(
            guard.category_over_budget(&ctx, ContextCategory::SystemPrompt),
            None
        );
    }

    #[test]
    fn test_token_budget_total_available_saturates_on_overflow() {
        let budget = TokenBudget {
            system_prompt: u32::MAX,
            tools_schema: 1,
            history: 0,
            bootstrap: 0,
            response_reserve: 1,
        };

        assert_eq!(budget.total_available(), u32::MAX);
        assert_eq!(budget.total_with_reserve(), u32::MAX);
    }

    #[test]
    fn test_guard_overflow_threshold_calculation_does_not_overflow() {
        let guard = ContextWindowGuard {
            budget: TokenBudget {
                system_prompt: u32::MAX,
                tools_schema: 0,
                history: 0,
                bootstrap: 0,
                response_reserve: 0,
            },
            critical_threshold: 99,
            ..ContextWindowGuard::new()
        };
        let ctx = make_ctx(3_900_000_000);

        assert!(matches!(
            guard.evaluate(&ctx),
            GuardVerdict::Overflow { tokens_over } if tokens_over > 0
        ));
    }
}
