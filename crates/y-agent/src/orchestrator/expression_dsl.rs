//! Expression DSL: shorthand syntax for workflow composition.
//!
//! Design reference: orchestrator-design.md §Expression DSL
//!
//! Supports two operators:
//! - `>>` for sequential composition
//! - `|` for parallel composition
//! - Parentheses for grouping
//!
//! Example: `search >> (analyze | score) >> summarize`
//!
//! The DSL compiles to the same internal `TaskDag` representation
//! used by TOML workflow definitions.

use std::collections::HashMap;
use std::fmt;
use std::hash::BuildHasher;

use crate::orchestrator::dag::{DagError, TaskDag, TaskNode, TaskPriority};

// ---------------------------------------------------------------------------
// Token types
// ---------------------------------------------------------------------------

/// A token produced by the DSL lexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A task reference (alphanumeric + underscore + hyphen).
    TaskRef(String),
    /// Sequential operator `>>`.
    Sequential,
    /// Parallel operator `|`.
    Parallel,
    /// Left parenthesis.
    LeftParen,
    /// Right parenthesis.
    RightParen,
}

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

/// Abstract syntax tree for parsed DSL expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DslWorkflow {
    /// A single task reference.
    Task(String),
    /// Sequential composition: execute children in order.
    Sequential(Vec<DslWorkflow>),
    /// Parallel composition: execute children concurrently.
    Parallel(Vec<DslWorkflow>),
}

impl fmt::Display for DslWorkflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DslWorkflow::Task(name) => write!(f, "{name}"),
            DslWorkflow::Sequential(steps) => {
                for (i, step) in steps.iter().enumerate() {
                    if i > 0 {
                        write!(f, " >> ")?;
                    }
                    // Wrap parallel children in parentheses to preserve semantics.
                    if matches!(step, DslWorkflow::Parallel(_)) {
                        write!(f, "({step})")?;
                    } else {
                        write!(f, "{step}")?;
                    }
                }
                Ok(())
            }
            DslWorkflow::Parallel(branches) => {
                for (i, branch) in branches.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{branch}")?;
                }
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from DSL tokenization or parsing.
#[derive(Debug, thiserror::Error)]
pub enum DslError {
    #[error("unexpected character '{ch}' at position {pos}")]
    UnexpectedChar { ch: char, pos: usize },
    #[error("unexpected end of expression at position {pos}")]
    UnexpectedEnd { pos: usize },
    #[error("unexpected token {token:?} at position {pos}")]
    UnexpectedToken { token: Token, pos: usize },
    #[error("mismatched parentheses at position {pos}")]
    MismatchedParens { pos: usize },
    #[error("empty expression")]
    EmptyExpression,
    #[error("duplicate task name '{name}' in expression")]
    DuplicateTaskName { name: String },
    #[error("template variable '{{{{{{name}}}}}}' contains invalid characters for a task name: '{value}'")]
    InvalidTemplateValue { name: String, value: String },
    #[error("DAG compilation error: {0}")]
    DagError(#[from] DagError),
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/// Tokenize a DSL expression string into tokens with their positions.
pub fn tokenize(input: &str) -> Result<Vec<(Token, usize)>, DslError> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => {
                i += 1;
            }
            b'(' => {
                tokens.push((Token::LeftParen, i));
                i += 1;
            }
            b')' => {
                tokens.push((Token::RightParen, i));
                i += 1;
            }
            b'|' => {
                tokens.push((Token::Parallel, i));
                i += 1;
            }
            b'>' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                    tokens.push((Token::Sequential, i));
                    i += 2;
                } else {
                    return Err(DslError::UnexpectedChar { ch: '>', pos: i });
                }
            }
            c if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
                {
                    i += 1;
                }
                let name = String::from_utf8_lossy(&bytes[start..i]).into_owned();
                tokens.push((Token::TaskRef(name), start));
            }
            _ => {
                // Handle multi-byte UTF-8 characters for error reporting.
                let ch = input[i..].chars().next().unwrap_or('?');
                return Err(DslError::UnexpectedChar { ch, pos: i });
            }
        }
    }

    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Parser (recursive descent)
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<(Token, usize)>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<(Token, usize)>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(tok, _)| tok)
    }

    fn advance(&mut self) -> Option<(Token, usize)> {
        if self.pos < self.tokens.len() {
            let (tok, pos) = self.tokens[self.pos].clone();
            self.pos += 1;
            Some((tok, pos))
        } else {
            None
        }
    }

    /// Input length (for error reporting at end of expression).
    fn input_len(&self) -> usize {
        self.tokens.last().map_or(0, |(_, pos)| *pos + 1)
    }

    /// Parse the full expression.
    /// Grammar:
    ///   expr     = parallel (">>" parallel)*
    ///   parallel = primary ("|" primary)*
    ///   primary  = `TASK_REF` | "(" expr ")"
    fn parse_expr(&mut self) -> Result<DslWorkflow, DslError> {
        let mut parts = vec![self.parse_parallel()?];

        while self.peek() == Some(&Token::Sequential) {
            self.advance(); // consume >>
            parts.push(self.parse_parallel()?);
        }

        if parts.len() == 1 {
            Ok(parts.remove(0))
        } else {
            Ok(DslWorkflow::Sequential(parts))
        }
    }

    fn parse_parallel(&mut self) -> Result<DslWorkflow, DslError> {
        let mut parts = vec![self.parse_primary()?];

        while self.peek() == Some(&Token::Parallel) {
            self.advance(); // consume |
            parts.push(self.parse_primary()?);
        }

        if parts.len() == 1 {
            Ok(parts.remove(0))
        } else {
            Ok(DslWorkflow::Parallel(parts))
        }
    }

    fn parse_primary(&mut self) -> Result<DslWorkflow, DslError> {
        match self.advance() {
            Some((Token::TaskRef(name), _)) => Ok(DslWorkflow::Task(name)),
            Some((Token::LeftParen, _)) => {
                let inner = self.parse_expr()?;
                match self.advance() {
                    Some((Token::RightParen, _)) => Ok(inner),
                    Some((_, pos)) => Err(DslError::MismatchedParens { pos }),
                    None => Err(DslError::MismatchedParens {
                        pos: self.input_len(),
                    }),
                }
            }
            Some((tok, pos)) => Err(DslError::UnexpectedToken { token: tok, pos }),
            None => Err(DslError::UnexpectedEnd {
                pos: self.input_len(),
            }),
        }
    }
}

/// Parse a DSL expression string into an AST.
pub fn parse(input: &str) -> Result<DslWorkflow, DslError> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err(DslError::EmptyExpression);
    }

    let mut parser = Parser::new(tokens);
    let result = parser.parse_expr()?;

    // Ensure all tokens were consumed.
    if parser.pos < parser.tokens.len() {
        let (tok, pos) = parser.tokens[parser.pos].clone();
        return Err(DslError::UnexpectedToken { token: tok, pos });
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// DSL → TaskDag compilation
// ---------------------------------------------------------------------------

impl DslWorkflow {
    /// Compile the DSL AST into a `TaskDag`.
    ///
    /// Sequential nodes create dependency chains.
    /// Parallel nodes create independent tasks that share a dependency
    /// barrier (a synthetic join node) for the next sequential step.
    ///
    /// Returns an error if the expression contains duplicate task names.
    pub fn to_task_dag(&self) -> Result<TaskDag, DslError> {
        let mut dag = TaskDag::new();
        self.compile(&mut dag, &[])?;
        Ok(dag)
    }

    /// Compile recursively. Returns the IDs of the "tail" tasks
    /// (tasks that should be depended upon by the next sequential step).
    fn compile(&self, dag: &mut TaskDag, deps: &[String]) -> Result<Vec<String>, DslError> {
        match self {
            DslWorkflow::Task(name) => {
                let id = name.clone();
                dag.add_task(TaskNode {
                    id: id.clone(),
                    name: name.clone(),
                    priority: TaskPriority::Normal,
                    dependencies: deps.to_vec(),
                })?;
                Ok(vec![id])
            }
            DslWorkflow::Sequential(steps) => {
                let mut current_deps = deps.to_vec();
                for step in steps {
                    current_deps = step.compile(dag, &current_deps)?;
                }
                Ok(current_deps)
            }
            DslWorkflow::Parallel(branches) => {
                let mut tail_ids = Vec::new();
                for branch in branches {
                    let tails = branch.compile(dag, deps)?;
                    tail_ids.extend(tails);
                }
                Ok(tail_ids)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Template expansion
// ---------------------------------------------------------------------------

/// Check if a string is a valid task name fragment (alphanumeric, underscore, hyphen).
fn is_valid_task_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Expand template variables in a DSL expression string.
///
/// Replaces `{{key}}` with the corresponding value from the variables map.
/// Returns an error if any substituted value contains characters that are
/// invalid in a task name (whitespace, operators, etc.).
pub fn expand_template<S: BuildHasher>(
    template: &str,
    variables: &HashMap<String, String, S>,
) -> Result<String, DslError> {
    let mut result = template.to_string();
    for (key, value) in variables {
        let placeholder = format!("{{{{{key}}}}}");
        if result.contains(&placeholder) && !is_valid_task_name(value) {
            return Err(DslError::InvalidTemplateValue {
                name: key.clone(),
                value: value.clone(),
            });
        }
        result = result.replace(&placeholder, value.as_str());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// T-P3-42-01: Tokenize simple sequential expression.
    #[test]
    fn test_tokenize_sequential() {
        let tokens: Vec<Token> = tokenize("a >> b >> c")
            .unwrap()
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        assert_eq!(
            tokens,
            vec![
                Token::TaskRef("a".into()),
                Token::Sequential,
                Token::TaskRef("b".into()),
                Token::Sequential,
                Token::TaskRef("c".into()),
            ]
        );
    }

    /// T-P3-42-02: Tokenize parallel expression.
    #[test]
    fn test_tokenize_parallel() {
        let tokens: Vec<Token> = tokenize("a | b | c")
            .unwrap()
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        assert_eq!(
            tokens,
            vec![
                Token::TaskRef("a".into()),
                Token::Parallel,
                Token::TaskRef("b".into()),
                Token::Parallel,
                Token::TaskRef("c".into()),
            ]
        );
    }

    /// T-P3-42-03: Parse sequential expression.
    #[test]
    fn test_parse_sequential() {
        let ast = parse("search >> analyze >> summarize").unwrap();
        assert_eq!(
            ast,
            DslWorkflow::Sequential(vec![
                DslWorkflow::Task("search".into()),
                DslWorkflow::Task("analyze".into()),
                DslWorkflow::Task("summarize".into()),
            ])
        );
    }

    /// T-P3-42-04: Parse parallel expression.
    #[test]
    fn test_parse_parallel() {
        let ast = parse("analyze | score").unwrap();
        assert_eq!(
            ast,
            DslWorkflow::Parallel(vec![
                DslWorkflow::Task("analyze".into()),
                DslWorkflow::Task("score".into()),
            ])
        );
    }

    /// T-P3-42-05: Parse nested expression with parentheses.
    #[test]
    fn test_parse_nested() {
        let ast = parse("search >> (analyze | score) >> summarize").unwrap();
        assert_eq!(
            ast,
            DslWorkflow::Sequential(vec![
                DslWorkflow::Task("search".into()),
                DslWorkflow::Parallel(vec![
                    DslWorkflow::Task("analyze".into()),
                    DslWorkflow::Task("score".into()),
                ]),
                DslWorkflow::Task("summarize".into()),
            ])
        );
    }

    /// T-P3-42-06: Compile sequential DSL to `TaskDag`.
    #[test]
    fn test_compile_sequential_dag() {
        let ast = parse("a >> b >> c").unwrap();
        let dag = ast.to_task_dag().unwrap();
        assert!(dag.validate().is_ok());

        // a is ready first
        let completed = HashSet::new();
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "a");

        // After a, b is ready
        let mut completed = HashSet::new();
        completed.insert("a".to_string());
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "b");

        // After a+b, c is ready
        completed.insert("b".to_string());
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "c");
    }

    /// T-P3-42-07: Compile parallel DSL to `TaskDag`.
    #[test]
    fn test_compile_parallel_dag() {
        let ast = parse("a | b | c").unwrap();
        let dag = ast.to_task_dag().unwrap();
        assert!(dag.validate().is_ok());

        // All three are ready simultaneously.
        let completed = HashSet::new();
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 3);
    }

    /// T-P3-42-08: Compile nested DSL: search >> (analyze | score) >> summarize.
    #[test]
    fn test_compile_nested_dag() {
        let ast = parse("search >> (analyze | score) >> summarize").unwrap();
        let dag = ast.to_task_dag().unwrap();
        assert!(dag.validate().is_ok());

        // Only search is ready first
        let completed = HashSet::new();
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "search");

        // After search: analyze and score are ready in parallel
        let mut completed = HashSet::new();
        completed.insert("search".to_string());
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 2);
        let names: HashSet<_> = ready.iter().map(|t| t.id.as_str()).collect();
        assert!(names.contains("analyze"));
        assert!(names.contains("score"));

        // After search + analyze + score: summarize is ready
        completed.insert("analyze".to_string());
        completed.insert("score".to_string());
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "summarize");
    }

    /// T-P3-42-09: Template variable expansion with valid value.
    #[test]
    fn test_template_expansion_valid() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("query".to_string(), "rust-async".to_string());
        let result = expand_template("search_{{query}} >> analyze", &vars).unwrap();
        assert_eq!(result, "search_rust-async >> analyze");
    }

    /// T-P3-42-10: Error on empty expression.
    #[test]
    fn test_parse_empty() {
        let result = parse("");
        assert!(result.is_err());
    }

    /// T-P3-42-11: Error on mismatched parentheses.
    #[test]
    fn test_parse_mismatched_parens() {
        let result = parse("(a >> b");
        assert!(result.is_err());
    }

    /// T-P3-42-12: Task names with hyphens and underscores.
    #[test]
    fn test_task_names_special_chars() {
        let ast = parse("web-search >> data_analysis").unwrap();
        assert_eq!(
            ast,
            DslWorkflow::Sequential(vec![
                DslWorkflow::Task("web-search".into()),
                DslWorkflow::Task("data_analysis".into()),
            ])
        );
    }

    /// T-P3-42-13: Duplicate task names produce `DuplicateTask` error.
    #[test]
    fn test_duplicate_task_names() {
        let ast = parse("a >> b >> a").unwrap();
        let result = ast.to_task_dag();
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(err_str.contains("duplicate"), "error: {err_str}");
    }

    /// T-P3-42-14: Template variable with whitespace is rejected.
    #[test]
    fn test_template_expansion_invalid_whitespace() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("query".to_string(), "rust async".to_string());
        let result = expand_template("search_{{query}} >> analyze", &vars);
        assert!(result.is_err());
    }

    /// T-P3-42-15: Template variable with operator characters is rejected.
    #[test]
    fn test_template_expansion_invalid_operator() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("name".to_string(), "a >> b".to_string());
        let result = expand_template("{{name}} >> c", &vars);
        assert!(result.is_err());
    }

    /// T-P3-42-16: Display round-trips simple sequential expression.
    #[test]
    fn test_display_sequential() {
        let ast = parse("a >> b >> c").unwrap();
        assert_eq!(ast.to_string(), "a >> b >> c");
    }

    /// T-P3-42-17: Display round-trips nested expression with parentheses.
    #[test]
    fn test_display_nested() {
        let ast = parse("search >> (analyze | score) >> summarize").unwrap();
        assert_eq!(ast.to_string(), "search >> (analyze | score) >> summarize");
    }

    /// T-P3-42-18: Single task expression.
    #[test]
    fn test_single_task() {
        let ast = parse("search").unwrap();
        assert_eq!(ast, DslWorkflow::Task("search".into()));
        let dag = ast.to_task_dag().unwrap();
        assert_eq!(dag.len(), 1);
    }

    /// T-P3-42-19: Leading operator is an error.
    #[test]
    fn test_leading_operator() {
        let result = parse(">> a");
        assert!(result.is_err());
    }

    /// T-P3-42-20: Adjacent operators are an error.
    #[test]
    fn test_adjacent_operators() {
        let result = parse("a >> >> b");
        assert!(result.is_err());
    }

    /// T-P3-42-21: Empty parentheses are an error.
    #[test]
    fn test_empty_parens() {
        let result = parse("()");
        assert!(result.is_err());
    }

    /// T-P3-42-22: Error positions include character offset.
    #[test]
    fn test_error_has_position() {
        let err = parse("a >> ").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("position"),
            "error should include position: {msg}"
        );
    }
}
