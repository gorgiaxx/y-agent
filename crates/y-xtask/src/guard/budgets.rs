//! Ratcheting quality budgets.
//!
//! A budget is a monotonically non-increasing ceiling, not a hard zero. Counts
//! above the ceiling fail; counts below it also fail, demanding the ceiling be
//! lowered. Without the second half a ratchet never actually ratchets.

use std::fmt;

/// Threshold above which a production file counts as oversized.
pub const OVERSIZED_FILE_LOC: usize = 1200;

/// A tracked code-quality metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Metric {
    /// `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!` in production code.
    Panics,
    /// Discarded results: `let _ = `, `.ok();`, `.unwrap_or_default()`.
    SwallowedErrors,
    /// Production files exceeding [`OVERSIZED_FILE_LOC`] lines.
    OversizedFiles,
    /// Cross-crate wildcard re-exports (`pub use y_foo::*`).
    WildcardReexports,
    /// Inline lint suppressions, forbidden by `AGENTS.md` 2.10.
    LintSuppressions,
}

impl Metric {
    /// Every metric, in stable order.
    pub const ALL: [Metric; 5] = [
        Metric::Panics,
        Metric::SwallowedErrors,
        Metric::OversizedFiles,
        Metric::WildcardReexports,
        Metric::LintSuppressions,
    ];

    /// Stable key used in `guards.toml`.
    pub fn key(self) -> &'static str {
        match self {
            Metric::Panics => "panics",
            Metric::SwallowedErrors => "swallowed_errors",
            Metric::OversizedFiles => "oversized_files",
            Metric::WildcardReexports => "wildcard_reexports",
            Metric::LintSuppressions => "lint_suppressions",
        }
    }

    /// Parse a `guards.toml` key.
    pub fn from_key(key: &str) -> Option<Metric> {
        Metric::ALL.into_iter().find(|metric| metric.key() == key)
    }

    /// One-line explanation shown when the budget is exceeded.
    pub fn rationale(self) -> &'static str {
        match self {
            Metric::Panics => "a panic in a harness aborts the user's session; return an error",
            Metric::SwallowedErrors => "a discarded result is a failure the operator never sees",
            Metric::OversizedFiles => "oversized files defeat review and reuse (AGENTS.md 2.11)",
            Metric::WildcardReexports => "wildcard re-exports erase crate boundaries",
            Metric::LintSuppressions => "fix the lint or move the rule to its owning config (2.10)",
        }
    }
}

impl fmt::Display for Metric {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.key())
    }
}

/// Drop the `#[cfg(test)]` tail of a Rust source file.
///
/// Budgets describe production code. Test modules legitimately use `unwrap` and
/// are excluded rather than granted a separate, unenforced allowance.
pub fn production_slice(source: &str) -> &str {
    match source.find("#[cfg(test)]") {
        Some(index) => &source[..index],
        None => source,
    }
}

/// Count non-overlapping occurrences of `needle` in `haystack`.
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.matches(needle).count()
}

/// Count wildcard re-exports of workspace crates: `pub use y_foo::*`.
fn count_wildcard_reexports(source: &str) -> usize {
    source
        .match_indices("pub use y_")
        .filter(|(index, _)| {
            let rest = &source[*index..];
            rest.find(';')
                .is_some_and(|end| rest[..end].trim_end().ends_with("::*"))
        })
        .count()
}

/// Count the occurrences of `metric` inside one Rust production slice.
pub fn count_rust(metric: Metric, source: &str) -> usize {
    match metric {
        Metric::Panics => {
            count_occurrences(source, ".unwrap()")
                + count_occurrences(source, ".expect(")
                + count_occurrences(source, "panic!(")
                + count_occurrences(source, "todo!(")
                + count_occurrences(source, "unimplemented!(")
        }
        Metric::SwallowedErrors => {
            count_occurrences(source, "let _ = ")
                + count_occurrences(source, ".ok();")
                + count_occurrences(source, ".unwrap_or_default()")
        }
        Metric::WildcardReexports => count_wildcard_reexports(source),
        Metric::LintSuppressions => count_occurrences(source, "#[allow(clippy::"),
        // File-level metric; counted by the walker, not by content.
        Metric::OversizedFiles => 0,
    }
}

/// Count inline lint suppressions in a TypeScript or TSX source file.
pub fn count_typescript_suppressions(source: &str) -> usize {
    count_occurrences(source, "eslint-disable") + count_occurrences(source, "@ts-ignore")
}

/// Outcome of comparing one metric against its recorded budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Count matches the budget exactly.
    Met,
    /// Count exceeds the budget.
    Exceeded {
        /// How many occurrences over budget.
        excess: usize,
    },
    /// Count is below the budget; the budget must be tightened.
    Stale {
        /// The value the budget should be lowered to.
        tighten_to: usize,
    },
}

/// Compare an observed `count` against its recorded `budget`.
pub fn verdict(count: usize, budget: usize) -> Verdict {
    match count.cmp(&budget) {
        std::cmp::Ordering::Equal => Verdict::Met,
        std::cmp::Ordering::Greater => Verdict::Exceeded {
            excess: count - budget,
        },
        std::cmp::Ordering::Less => Verdict::Stale { tighten_to: count },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_slice_drops_test_module() {
        let source = "fn a() {}\n#[cfg(test)]\nmod tests { fn b() { x.unwrap() } }";
        assert_eq!(production_slice(source), "fn a() {}\n");
    }

    #[test]
    fn production_slice_keeps_files_without_tests() {
        let source = "fn a() {}\n";
        assert_eq!(production_slice(source), source);
    }

    #[test]
    fn panic_metric_counts_every_form() {
        let source = "a.unwrap(); b.expect(\"x\"); panic!(\"y\"); todo!(); unimplemented!();";
        assert_eq!(count_rust(Metric::Panics, source), 5);
    }

    #[test]
    fn panic_metric_ignores_unwrap_or() {
        assert_eq!(count_rust(Metric::Panics, "a.unwrap_or(3)"), 0);
    }

    #[test]
    fn swallowed_error_metric_counts_every_form() {
        let source = "let _ = f();\ng().ok();\nh().unwrap_or_default()";
        assert_eq!(count_rust(Metric::SwallowedErrors, source), 3);
    }

    #[test]
    fn wildcard_reexport_requires_glob_and_workspace_prefix() {
        assert_eq!(
            count_rust(Metric::WildcardReexports, "pub use y_core::*;"),
            1
        );
        assert_eq!(
            count_rust(Metric::WildcardReexports, "pub use y_core::Message;"),
            0
        );
        assert_eq!(
            count_rust(Metric::WildcardReexports, "pub use serde::*;"),
            0
        );
    }

    #[test]
    fn lint_suppression_metric_covers_rust_and_typescript() {
        assert_eq!(
            count_rust(Metric::LintSuppressions, "#[allow(clippy::too_many_lines)]"),
            1
        );
        assert_eq!(
            count_rust(Metric::LintSuppressions, "#[allow(dead_code)]"),
            0
        );
        assert_eq!(
            count_typescript_suppressions("// eslint-disable-next-line\n// @ts-ignore"),
            2
        );
    }

    #[test]
    fn metric_keys_round_trip() {
        for metric in Metric::ALL {
            assert_eq!(Metric::from_key(metric.key()), Some(metric));
        }
        assert_eq!(Metric::from_key("nope"), None);
    }

    #[test]
    fn verdict_distinguishes_over_under_and_exact() {
        assert_eq!(verdict(10, 10), Verdict::Met);
        assert_eq!(verdict(12, 10), Verdict::Exceeded { excess: 2 });
        assert_eq!(verdict(7, 10), Verdict::Stale { tighten_to: 7 });
    }
}
