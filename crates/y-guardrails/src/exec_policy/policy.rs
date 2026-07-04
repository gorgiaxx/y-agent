//! Policy: indexed rule collection with matching and merge.
//!
//! Rules are indexed by their first token in a `MultiMap` for O(1) lookup.
//! When multiple rules match, the strictest decision wins
//! (`Deny` > `Ask` > `Allow`).

use std::collections::HashMap;
use std::sync::Arc;

use multimap::MultiMap;
use serde::{Deserialize, Serialize};

use crate::exec_policy::decision::ExecDecision;
use crate::exec_policy::error::{ExecPolicyError, ExecPolicyResult};
use crate::exec_policy::rule::{PrefixRule, Rule, RuleMatch, RuleRef};

/// An indexed collection of rules that can evaluate commands.
#[derive(Clone, Debug, Default)]
pub struct Policy {
    /// Rules indexed by first token.
    rules_by_program: MultiMap<String, RuleRef>,
}

impl Policy {
    /// Create a policy from a list of rules (indexed internally).
    pub fn from_rules(rules: Vec<RuleRef>) -> Self {
        let mut rules_by_program = MultiMap::new();
        for rule in rules {
            rules_by_program.insert(rule.program().to_string(), rule);
        }
        Self { rules_by_program }
    }

    /// Create from raw parts (used by the parser).
    pub fn from_parts(rules_by_program: MultiMap<String, RuleRef>) -> Self {
        Self { rules_by_program }
    }

    /// Returns all matches for `cmd`, or an empty vec if none.
    ///
    /// Looks up rules by the first token, then checks each for a prefix match.
    pub fn matches_for_command(&self, cmd: &[String]) -> Vec<RuleMatch> {
        let Some(first) = cmd.first() else {
            return Vec::new();
        };
        let Some(rules) = self.rules_by_program.get_vec(first) else {
            return Vec::new();
        };
        rules.iter().filter_map(|rule| rule.matches(cmd)).collect()
    }

    /// Returns `true` if any rule matches `cmd`.
    pub fn has_match(&self, cmd: &[String]) -> bool {
        !self.matches_for_command(cmd).is_empty()
    }

    /// Evaluate `cmd` against the policy and return the strictest decision.
    ///
    /// Returns `None` if no rule matches (caller should apply heuristics/fallback).
    pub fn evaluate(&self, cmd: &[String]) -> Option<Evaluation> {
        let matches = self.matches_for_command(cmd);
        if matches.is_empty() {
            return None;
        }
        Some(Evaluation::from_matches(matches))
    }

    /// Merge `overlay` onto `self`, returning a new policy.
    ///
    /// Rules are unioned: rules from `overlay` with the same first token are
    /// appended to `self`'s rules for that token.
    #[must_use]
    pub fn merge_overlay(&self, overlay: &Self) -> Self {
        let mut merged = self.rules_by_program.clone();
        for (key, rules) in &overlay.rules_by_program {
            for rule in rules {
                merged.insert(key.clone(), Arc::clone(rule));
            }
        }
        Self {
            rules_by_program: merged,
        }
    }

    /// Iterate over all rules (for debugging/inspection).
    pub fn all_rules(&self) -> impl Iterator<Item = &RuleRef> {
        self.rules_by_program.iter().map(|(_, rule)| rule)
    }

    /// Count of rules.
    pub fn rule_count(&self) -> usize {
        self.rules_by_program.len()
    }
}

/// Result of evaluating a command against the policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evaluation {
    /// The final decision (strictest of all matches).
    pub decision: ExecDecision,
    /// All matches that contributed.
    pub matches: Vec<RuleMatch>,
}

impl Evaluation {
    /// Aggregate matches into a single evaluation (strictest wins).
    pub fn from_matches(matches: Vec<RuleMatch>) -> Self {
        let decision = matches
            .iter()
            .map(RuleMatch::decision)
            .max()
            .unwrap_or(ExecDecision::Allow);
        Self { decision, matches }
    }

    /// The justification from the match that determined the final decision.
    pub fn determining_justification(&self) -> Option<&str> {
        self.matches
            .iter()
            .filter(|m| m.decision() == self.decision)
            .find_map(RuleMatch::justification)
    }
}

// -----------------------------------------------------------------------
// Example validation (used by parser at parse time)
// -----------------------------------------------------------------------

/// Validate that each `match` example is matched by at least one rule.
pub fn validate_match_examples(policy: &Policy, matches: &[Vec<String>]) -> ExecPolicyResult<()> {
    for example in matches {
        if !policy.has_match(example) {
            return Err(ExecPolicyError::MatchExampleFailed {
                example: example.clone(),
            });
        }
    }
    Ok(())
}

/// Validate that no `not_match` example is matched by any rule.
pub fn validate_not_match_examples(
    policy: &Policy,
    not_matches: &[Vec<String>],
) -> ExecPolicyResult<()> {
    for example in not_matches {
        if policy.has_match(example) {
            return Err(ExecPolicyError::NotMatchExampleFailed {
                example: example.clone(),
            });
        }
    }
    Ok(())
}

/// Build a temporary policy from a slice of prefix rules for example validation.
pub fn policy_from_prefix_rules(rules: &[Arc<PrefixRule>]) -> Policy {
    let rule_refs: Vec<RuleRef> = rules.iter().map(|r| Arc::clone(r) as RuleRef).collect();
    Policy::from_rules(rule_refs)
}

/// Convert a flat list of prefix rules to an indexed `HashMap` (for local validation).
pub fn prefix_rules_by_program(rules: &[Arc<PrefixRule>]) -> HashMap<String, Vec<Arc<PrefixRule>>> {
    let mut map: HashMap<String, Vec<Arc<PrefixRule>>> = HashMap::new();
    for rule in rules {
        map.entry(rule.program().to_string())
            .or_default()
            .push(Arc::clone(rule));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_policy::rule::{PatternToken, PrefixPattern, PrefixRule};

    fn make_rule(first: &str, rest: Vec<PatternToken>, decision: ExecDecision) -> RuleRef {
        Arc::new(PrefixRule {
            pattern: PrefixPattern {
                first: Arc::from(first),
                rest: rest.into(),
            },
            decision,
            justification: None,
        })
    }

    #[test]
    fn empty_policy_no_match() {
        let policy = Policy::default();
        assert!(policy.evaluate(&["git".into(), "push".into()]).is_none());
    }

    #[test]
    fn single_rule_matches() {
        let rule = make_rule(
            "cargo",
            vec![PatternToken::Single("test".into())],
            ExecDecision::Allow,
        );
        let policy = Policy::from_rules(vec![rule]);
        let eval = policy.evaluate(&["cargo".into(), "test".into()]).unwrap();
        assert_eq!(eval.decision, ExecDecision::Allow);
    }

    #[test]
    fn strictest_wins() {
        let allow = make_rule("git", vec![], ExecDecision::Allow);
        let deny = make_rule("git", vec![], ExecDecision::Deny);
        let policy = Policy::from_rules(vec![allow, deny]);
        let eval = policy.evaluate(&["git".into()]).unwrap();
        assert_eq!(eval.decision, ExecDecision::Deny);
    }

    #[test]
    fn no_match_different_first_token() {
        let rule = make_rule("git", vec![], ExecDecision::Allow);
        let policy = Policy::from_rules(vec![rule]);
        assert!(policy.evaluate(&["npm".into()]).is_none());
    }

    #[test]
    fn merge_overlay_unions_rules() {
        let base = make_rule("git", vec![], ExecDecision::Allow);
        let overlay_rule = make_rule("npm", vec![], ExecDecision::Ask);
        let base_policy = Policy::from_rules(vec![base]);
        let overlay_policy = Policy::from_rules(vec![overlay_rule]);
        let merged = base_policy.merge_overlay(&overlay_policy);
        assert_eq!(merged.rule_count(), 2);
        assert!(merged.has_match(&["git".into()]));
        assert!(merged.has_match(&["npm".into()]));
    }

    #[test]
    fn validate_match_examples_pass() {
        let rule = make_rule("git", vec![], ExecDecision::Allow);
        let policy = Policy::from_rules(vec![rule]);
        validate_match_examples(&policy, &[vec!["git".into()]]).unwrap();
    }

    #[test]
    fn validate_match_examples_fail() {
        let rule = make_rule("git", vec![], ExecDecision::Allow);
        let policy = Policy::from_rules(vec![rule]);
        assert!(validate_match_examples(&policy, &[vec!["npm".into()]]).is_err());
    }

    #[test]
    fn validate_not_match_examples_pass() {
        let rule = make_rule("git", vec![], ExecDecision::Allow);
        let policy = Policy::from_rules(vec![rule]);
        validate_not_match_examples(&policy, &[vec!["npm".into()]]).unwrap();
    }

    #[test]
    fn validate_not_match_examples_fail() {
        let rule = make_rule("git", vec![], ExecDecision::Allow);
        let policy = Policy::from_rules(vec![rule]);
        assert!(validate_not_match_examples(&policy, &[vec!["git".into()]]).is_err());
    }
}
