//! Rule types for the exec policy engine.
//!
//! A [`PrefixRule`] matches a command by its leading tokens. The first token
//! is fixed (used as a lookup key); subsequent tokens support alternatives via
//! [`PatternToken::Alts`].

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::exec_policy::decision::ExecDecision;

/// Matches a single command token: a fixed string or one of several alternatives.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum PatternToken {
    /// A fixed string that must match exactly.
    Single(String),
    /// Any of the listed alternatives.
    Alts(Vec<String>),
}

impl PatternToken {
    /// Returns `true` if `token` satisfies this pattern.
    pub fn matches(&self, token: &str) -> bool {
        match self {
            Self::Single(expected) => expected == token,
            Self::Alts(alternatives) => alternatives.iter().any(|alt| alt == token),
        }
    }

    /// All string alternatives this token accepts.
    pub fn alternatives(&self) -> &[String] {
        match self {
            Self::Single(expected) => std::slice::from_ref(expected),
            Self::Alts(alternatives) => alternatives,
        }
    }
}

/// Prefix matcher: first token fixed, rest support alternatives.
///
/// The first token is fixed because the policy indexes rules by the first
/// token for O(1) lookup.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PrefixPattern {
    /// Fixed first token (the command name).
    pub first: Arc<str>,
    /// Remaining pattern tokens.
    pub rest: Arc<[PatternToken]>,
}

impl PrefixPattern {
    /// Returns the matched prefix if `cmd` starts with this pattern, else `None`.
    ///
    /// The returned vector is `cmd[..pattern_len]` — the portion of the command
    /// consumed by the match.
    pub fn matches_prefix(&self, cmd: &[String]) -> Option<Vec<String>> {
        let pattern_length = self.rest.len() + 1;
        if cmd.len() < pattern_length || cmd[0] != self.first.as_ref() {
            return None;
        }

        for (pattern_token, cmd_token) in self.rest.iter().zip(&cmd[1..pattern_length]) {
            if !pattern_token.matches(cmd_token) {
                return None;
            }
        }

        Some(cmd[..pattern_length].to_vec())
    }

    /// Number of tokens in the pattern (including the first fixed token).
    pub fn len(&self) -> usize {
        self.rest.len() + 1
    }

    /// `true` if the pattern is empty (always false — first token is required).
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// A prefix rule with a decision and optional justification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefixRule {
    /// The prefix pattern to match.
    pub pattern: PrefixPattern,
    /// The decision if this rule matches.
    pub decision: ExecDecision,
    /// Human-readable explanation.
    pub justification: Option<String>,
}

/// What kind of rule produced a match.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RuleMatch {
    /// Matched by a prefix rule.
    PrefixRuleMatch {
        /// The portion of the command consumed by the match.
        matched_prefix: Vec<String>,
        /// The decision from the matching rule.
        decision: ExecDecision,
        /// Optional justification from the rule.
        justification: Option<String>,
    },
}

impl RuleMatch {
    /// The decision from this match.
    pub const fn decision(&self) -> ExecDecision {
        match self {
            Self::PrefixRuleMatch { decision, .. } => *decision,
        }
    }

    /// The justification, if any.
    pub fn justification(&self) -> Option<&str> {
        match self {
            Self::PrefixRuleMatch { justification, .. } => justification.as_deref(),
        }
    }
}

/// Trait for abstract rule matching (extensible to future rule types).
pub trait Rule: std::any::Any + std::fmt::Debug + Send + Sync {
    /// The first token this rule keys on (for indexing).
    fn program(&self) -> &str;

    /// Returns a match if `cmd` starts with this rule's pattern, else `None`.
    fn matches(&self, cmd: &[String]) -> Option<RuleMatch>;
}

/// Type-erased rule reference.
pub type RuleRef = Arc<dyn Rule>;

impl Rule for PrefixRule {
    fn program(&self) -> &str {
        self.pattern.first.as_ref()
    }

    fn matches(&self, cmd: &[String]) -> Option<RuleMatch> {
        self.pattern
            .matches_prefix(cmd)
            .map(|matched_prefix| RuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision: self.decision,
                justification: self.justification.clone(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_token_single_matches() {
        let tok = PatternToken::Single("install".to_string());
        assert!(tok.matches("install"));
        assert!(!tok.matches("ci"));
    }

    #[test]
    fn pattern_token_alts_matches() {
        let tok = PatternToken::Alts(vec!["install".to_string(), "ci".to_string()]);
        assert!(tok.matches("install"));
        assert!(tok.matches("ci"));
        assert!(!tok.matches("run"));
    }

    #[test]
    fn prefix_pattern_exact_match() {
        let pat = PrefixPattern {
            first: Arc::from("git"),
            rest: Arc::from([PatternToken::Single("push".to_string())]),
        };
        assert_eq!(
            pat.matches_prefix(&["git".into(), "push".into(), "origin".into()]),
            Some(vec!["git".to_string(), "push".to_string()])
        );
    }

    #[test]
    fn prefix_pattern_no_match_wrong_first() {
        let pat = PrefixPattern {
            first: Arc::from("git"),
            rest: Arc::from([PatternToken::Single("push".to_string())]),
        };
        assert_eq!(pat.matches_prefix(&["npm".into(), "push".into()]), None);
    }

    #[test]
    fn prefix_pattern_no_match_too_short() {
        let pat = PrefixPattern {
            first: Arc::from("git"),
            rest: Arc::from([PatternToken::Single("push".to_string())]),
        };
        assert_eq!(pat.matches_prefix(&["git".into()]), None);
    }

    #[test]
    fn prefix_pattern_alts() {
        let pat = PrefixPattern {
            first: Arc::from("npm"),
            rest: Arc::from([PatternToken::Alts(vec![
                "install".to_string(),
                "ci".to_string(),
            ])]),
        };
        assert!(pat
            .matches_prefix(&["npm".into(), "install".into()])
            .is_some());
        assert!(pat.matches_prefix(&["npm".into(), "ci".into()]).is_some());
        assert!(pat.matches_prefix(&["npm".into(), "run".into()]).is_none());
    }

    #[test]
    fn prefix_rule_matches_and_returns_decision() {
        let rule = PrefixRule {
            pattern: PrefixPattern {
                first: Arc::from("cargo"),
                rest: Arc::from([PatternToken::Single("test".to_string())]),
            },
            decision: ExecDecision::Allow,
            justification: Some("tests are safe".to_string()),
        };
        let m = rule.matches(&["cargo".into(), "test".into()]).unwrap();
        assert_eq!(m.decision(), ExecDecision::Allow);
        assert_eq!(m.justification(), Some("tests are safe"));
    }

    #[test]
    fn prefix_rule_no_match() {
        let rule = PrefixRule {
            pattern: PrefixPattern {
                first: Arc::from("cargo"),
                rest: Arc::from([PatternToken::Single("test".to_string())]),
            },
            decision: ExecDecision::Allow,
            justification: None,
        };
        assert!(rule.matches(&["cargo".into(), "build".into()]).is_none());
    }
}
