//! Starlark parser for exec policy files.
//!
//! Policy files use a small subset of the Starlark language (the Python
//! dialect used by Bazel). One builtin function is available:
//!
//! - `prefix_rule(pattern, decision, match, not_match, justification)`
//!
//! Example policy file:
//!
//! ```python
//! prefix_rule(
//!     pattern = ["git", ["push", "commit"]],
//!     decision = "ask",
//!     match = [["git", "push"]],
//!     not_match = [["git", "status"]],
//!     justification = "review before push or commit",
//! )
//! ```

use std::cell::RefCell;
use std::cell::RefMut;
use std::sync::Arc;

use starlark::any::ProvidesStaticType;
use starlark::collections::SmallMap;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::AstModule;
use starlark::syntax::Dialect;
use starlark::values::list::ListRef;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::Value;

use crate::exec_policy::decision::ExecDecision;
use crate::exec_policy::error::{ExecPolicyError, ExecPolicyResult};
use crate::exec_policy::policy::{validate_match_examples, validate_not_match_examples, Policy};
use crate::exec_policy::rule::{PatternToken, PrefixPattern, PrefixRule, RuleRef};

/// Parses Starlark policy files into a [`Policy`].
pub struct PolicyParser {
    builder: RefCell<PolicyBuilder>,
}

impl Default for PolicyParser {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyParser {
    /// Create a new empty parser.
    pub fn new() -> Self {
        Self {
            builder: RefCell::new(PolicyBuilder::new()),
        }
    }

    /// Parse a policy file's contents.
    ///
    /// `policy_identifier` is used for error messages (typically the file path).
    /// May be called multiple times; rules accumulate.
    pub fn parse(
        &mut self,
        policy_identifier: &str,
        policy_file_contents: &str,
    ) -> ExecPolicyResult<()> {
        let pending_validation_count = self.builder.borrow().pending_example_validations.len();
        let mut dialect = Dialect::Extended.clone();
        dialect.enable_f_strings = true;
        let ast = AstModule::parse(
            policy_identifier,
            policy_file_contents.to_string(),
            &dialect,
        )
        .map_err(|e| ExecPolicyError::Starlark(anyhow::anyhow!(e)))?;
        let globals = GlobalsBuilder::standard().with(policy_builtins).build();
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&self.builder);
            eval.eval_module(ast, &globals)
                .map(|_| ())
                .map_err(|e| ExecPolicyError::Starlark(anyhow::anyhow!(e)))
        })?;
        self.builder
            .borrow()
            .validate_pending_examples_from(pending_validation_count)?;
        Ok(())
    }

    /// Consume the parser and return the built [`Policy`].
    pub fn build(self) -> Policy {
        self.builder.into_inner().build()
    }
}

// -----------------------------------------------------------------------
// Internal builder
// -----------------------------------------------------------------------

#[derive(Debug, ProvidesStaticType)]
struct PolicyBuilder {
    rules_by_program: SmallMap<String, Vec<RuleRef>>,
    pending_example_validations: Vec<PendingExampleValidation>,
}

impl PolicyBuilder {
    fn new() -> Self {
        Self {
            rules_by_program: SmallMap::new(),
            pending_example_validations: Vec::new(),
        }
    }

    fn add_rule(&mut self, rule: RuleRef) {
        self.rules_by_program
            .entry(rule.program().to_string())
            .or_default()
            .push(rule);
    }

    fn add_pending_example_validation(
        &mut self,
        rules: Vec<RuleRef>,
        matches: Vec<Vec<String>>,
        not_matches: Vec<Vec<String>>,
        location: Option<String>,
    ) {
        self.pending_example_validations
            .push(PendingExampleValidation {
                rules,
                matches,
                not_matches,
                location,
            });
    }

    fn validate_pending_examples_from(&self, start: usize) -> ExecPolicyResult<()> {
        for validation in &self.pending_example_validations[start..] {
            let policy = Policy::from_rules(validation.rules.clone());
            validate_not_match_examples(&policy, &validation.not_matches)
                .map_err(attach_location(validation.location.as_deref()))?;
            validate_match_examples(&policy, &validation.matches)
                .map_err(attach_location(validation.location.as_deref()))?;
        }
        Ok(())
    }

    fn build(self) -> Policy {
        let mut multi: multimap::MultiMap<String, RuleRef> = multimap::MultiMap::new();
        for (key, rules) in self.rules_by_program {
            for rule in rules {
                multi.insert(key.clone(), rule);
            }
        }
        Policy::from_parts(multi)
    }
}

#[derive(Debug)]
struct PendingExampleValidation {
    rules: Vec<RuleRef>,
    matches: Vec<Vec<String>>,
    not_matches: Vec<Vec<String>>,
    location: Option<String>,
}

fn attach_location(location: Option<&str>) -> impl Fn(ExecPolicyError) -> ExecPolicyError + '_ {
    move |err| match location {
        Some(loc) => ExecPolicyError::InvalidRule(format!("{loc}: {err}")),
        None => err,
    }
}

fn policy_builder<'a>(eval: &Evaluator<'_, 'a, '_>) -> RefMut<'a, PolicyBuilder> {
    #[expect(clippy::expect_used)]
    eval.extra
        .as_ref()
        .expect("policy_builder requires Evaluator.extra to be populated")
        .downcast_ref::<RefCell<PolicyBuilder>>()
        .expect("Evaluator.extra must contain a PolicyBuilder")
        .borrow_mut()
}

// -----------------------------------------------------------------------
// Starlark builtins
// -----------------------------------------------------------------------

#[starlark_module]
fn policy_builtins(builder: &mut GlobalsBuilder) {
    /// Define a prefix-based command approval rule.
    ///
    /// `pattern` is a list of tokens. The first token is fixed (the command
    /// name); subsequent tokens are either a string (exact match) or a list
    /// of strings (alternatives). `decision` defaults to `"allow"`. `match`
    /// and `not_match` are example commands validated at parse time.
    fn prefix_rule<'v>(
        pattern: UnpackList<Value<'v>>,
        decision: Option<&'v str>,
        r#match: Option<UnpackList<Value<'v>>>,
        not_match: Option<UnpackList<Value<'v>>>,
        justification: Option<&'v str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let decision = match decision {
            Some(raw) => ExecDecision::parse(raw).map_err(anyhow::Error::from)?,
            None => ExecDecision::Allow,
        };

        let justification = match justification {
            Some(raw) if raw.trim().is_empty() => {
                return Err(anyhow::anyhow!("prefix_rule justification cannot be empty"));
            }
            Some(raw) => Some(raw.to_string()),
            None => None,
        };

        let pattern_tokens = parse_pattern(pattern)?;

        let matches: Vec<Vec<String>> =
            r#match.map(parse_examples).transpose()?.unwrap_or_default();
        let not_matches: Vec<Vec<String>> = not_match
            .map(parse_examples)
            .transpose()?
            .unwrap_or_default();
        let location = eval.call_stack_top_location().map(|span| span.to_string());

        let (first_token, remaining_tokens) = pattern_tokens
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("pattern cannot be empty"))?;

        let rest: Arc<[PatternToken]> = remaining_tokens.to_vec().into();

        let rules: Vec<RuleRef> = first_token
            .alternatives()
            .iter()
            .map(|head| {
                Arc::new(PrefixRule {
                    pattern: PrefixPattern {
                        first: Arc::from(head.as_str()),
                        rest: Arc::clone(&rest),
                    },
                    decision,
                    justification: justification.clone(),
                }) as RuleRef
            })
            .collect();

        let mut builder = policy_builder(eval);
        builder.add_pending_example_validation(rules.clone(), matches, not_matches, location);
        for rule in rules {
            builder.add_rule(rule);
        }
        Ok(NoneType)
    }
}

fn parse_pattern(pattern: UnpackList<Value<'_>>) -> anyhow::Result<Vec<PatternToken>> {
    let tokens: Vec<PatternToken> = pattern
        .items
        .into_iter()
        .map(parse_pattern_token)
        .collect::<anyhow::Result<_>>()?;
    if tokens.is_empty() {
        return Err(anyhow::anyhow!("pattern cannot be empty"));
    }
    Ok(tokens)
}

fn parse_pattern_token(value: Value<'_>) -> anyhow::Result<PatternToken> {
    if let Some(s) = value.unpack_str() {
        Ok(PatternToken::Single(s.to_string()))
    } else if let Some(list) = ListRef::from_value(value) {
        let tokens: Vec<String> = list
            .content()
            .iter()
            .map(|v| {
                v.unpack_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("pattern alternative must be a string"))
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(PatternToken::Alts(tokens))
    } else {
        Err(anyhow::anyhow!(
            "pattern token must be a string or list of strings"
        ))
    }
}

fn parse_examples(examples: UnpackList<Value<'_>>) -> anyhow::Result<Vec<Vec<String>>> {
    let mut result = Vec::new();
    for item in examples.items {
        if let Some(list) = ListRef::from_value(item) {
            let tokens: Vec<String> = list
                .content()
                .iter()
                .map(|v| {
                    v.unpack_str()
                        .map(str::to_string)
                        .ok_or_else(|| anyhow::anyhow!("example must be a list of strings"))
                })
                .collect::<anyhow::Result<_>>()?;
            result.push(tokens);
        } else if let Some(s) = item.unpack_str() {
            // Allow a bare string as a single-token example.
            result.push(vec![s.to_string()]);
        } else {
            return Err(anyhow::anyhow!("example must be a list or string"));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_prefix_rule() {
        let mut parser = PolicyParser::new();
        parser
            .parse(
                "test.policy",
                r#"
prefix_rule(
    pattern = ["cargo", "test"],
    decision = "allow",
)
"#,
            )
            .unwrap();
        let policy = parser.build();
        let eval = policy.evaluate(&["cargo".into(), "test".into()]).unwrap();
        assert_eq!(eval.decision, ExecDecision::Allow);
    }

    #[test]
    fn parse_prefix_rule_with_alternatives() {
        let mut parser = PolicyParser::new();
        parser
            .parse(
                "test.policy",
                r#"
prefix_rule(
    pattern = ["npm", ["install", "ci"]],
    decision = "ask",
)
"#,
            )
            .unwrap();
        let policy = parser.build();
        assert!(policy.evaluate(&["npm".into(), "install".into()]).is_some());
        assert!(policy.evaluate(&["npm".into(), "ci".into()]).is_some());
        assert!(policy.evaluate(&["npm".into(), "run".into()]).is_none());
    }

    #[test]
    fn parse_default_decision_is_allow() {
        let mut parser = PolicyParser::new();
        parser
            .parse("test.policy", r#"prefix_rule(pattern = ["ls"])"#)
            .unwrap();
        let policy = parser.build();
        let eval = policy.evaluate(&["ls".into()]).unwrap();
        assert_eq!(eval.decision, ExecDecision::Allow);
    }

    #[test]
    fn parse_match_examples_validated() {
        let mut parser = PolicyParser::new();
        parser
            .parse(
                "test.policy",
                r#"
prefix_rule(
    pattern = ["git", "push"],
    decision = "deny",
    match = [["git", "push"]],
)
"#,
            )
            .unwrap();
        let policy = parser.build();
        assert_eq!(
            policy
                .evaluate(&["git".into(), "push".into()])
                .unwrap()
                .decision,
            ExecDecision::Deny
        );
    }

    #[test]
    fn parse_match_example_fails_when_no_match() {
        let mut parser = PolicyParser::new();
        let result = parser.parse(
            "test.policy",
            r#"
prefix_rule(
    pattern = ["git", "push"],
    decision = "deny",
    match = [["npm", "install"]],
)
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_not_match_example_fails_when_matches() {
        let mut parser = PolicyParser::new();
        let result = parser.parse(
            "test.policy",
            r#"
prefix_rule(
    pattern = ["git", "push"],
    decision = "deny",
    not_match = [["git", "push"]],
)
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_justification() {
        let mut parser = PolicyParser::new();
        parser
            .parse(
                "test.policy",
                r#"
prefix_rule(
    pattern = ["rm"],
    decision = "deny",
    justification = "never allow rm",
)
"#,
            )
            .unwrap();
        let policy = parser.build();
        let eval = policy.evaluate(&["rm".into()]).unwrap();
        assert_eq!(eval.decision, ExecDecision::Deny);
        assert_eq!(eval.determining_justification(), Some("never allow rm"));
    }

    #[test]
    fn parse_empty_justification_errors() {
        let mut parser = PolicyParser::new();
        let result = parser.parse(
            "test.policy",
            r#"prefix_rule(pattern = ["ls"], justification = "")"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_multiple_rules_accumulate() {
        let mut parser = PolicyParser::new();
        parser
            .parse(
                "test.policy",
                r#"
prefix_rule(pattern = ["ls"], decision = "allow")
prefix_rule(pattern = ["cat"], decision = "allow")
"#,
            )
            .unwrap();
        let policy = parser.build();
        assert!(policy.has_match(&["ls".into()]));
        assert!(policy.has_match(&["cat".into()]));
    }

    #[test]
    fn parse_strictest_wins_across_rules() {
        let mut parser = PolicyParser::new();
        parser
            .parse(
                "test.policy",
                r#"
prefix_rule(pattern = ["git"], decision = "allow")
prefix_rule(pattern = ["git"], decision = "deny")
"#,
            )
            .unwrap();
        let policy = parser.build();
        let eval = policy.evaluate(&["git".into()]).unwrap();
        assert_eq!(eval.decision, ExecDecision::Deny);
    }

    #[test]
    fn parse_string_match_example_shorthand() {
        // A bare string in match= should be treated as a single-token example.
        let mut parser = PolicyParser::new();
        parser
            .parse(
                "test.policy",
                r#"
prefix_rule(
    pattern = ["ls"],
    decision = "allow",
    match = ["ls"],
)
"#,
            )
            .unwrap();
    }

    #[test]
    fn parse_bare_string_not_match_example() {
        let mut parser = PolicyParser::new();
        let result = parser.parse(
            "test.policy",
            r#"
prefix_rule(
    pattern = ["ls"],
    decision = "allow",
    not_match = ["cat"],
)
"#,
        );
        assert!(result.is_ok());
    }
}
