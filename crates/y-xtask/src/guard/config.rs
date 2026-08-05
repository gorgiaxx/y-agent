//! Persistence for the ratchet baselines recorded in `guards.toml`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{Context, Result};

use crate::guard::budgets::Metric;

/// Header written above a regenerated `guards.toml`.
const HEADER: &str = "\
# Ratcheting guard baselines, enforced by `cargo run -p y-xtask -- guard all`.
#
# These numbers are debt ceilings, not targets. They may only decrease. The guard
# fails when a count rises above its ceiling AND when a count falls below it
# without the ceiling being lowered, so improvements are locked in rather than
# silently reclaimed. Regenerate with `guard budgets --update`.
";

/// Parsed contents of `guards.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardConfig {
    /// Ceiling per quality metric.
    pub budgets: BTreeMap<String, usize>,
    /// Pre-existing layering debt: crate name -> tolerated illegal dependencies.
    pub layer_exceptions: BTreeMap<String, Vec<String>>,
}

impl GuardConfig {
    /// Parse `guards.toml` text.
    pub fn parse(text: &str) -> Result<GuardConfig> {
        let value: toml::Value = text.parse().context("guards.toml is not valid TOML")?;

        let mut budgets = BTreeMap::new();
        if let Some(table) = value.get("budgets").and_then(toml::Value::as_table) {
            for (key, entry) in table {
                Metric::from_key(key).with_context(|| {
                    format!("guards.toml records an unknown budget `{key}`; remove it")
                })?;
                let count = entry
                    .as_integer()
                    .and_then(|number| usize::try_from(number).ok())
                    .with_context(|| format!("budget `{key}` must be a non-negative integer"))?;
                budgets.insert(key.clone(), count);
            }
        }

        let mut layer_exceptions = BTreeMap::new();
        if let Some(table) = value
            .get("layers")
            .and_then(|layers| layers.get("exceptions"))
            .and_then(toml::Value::as_table)
        {
            for (krate, entry) in table {
                let list = entry
                    .as_array()
                    .with_context(|| format!("layer exception `{krate}` must be an array"))?
                    .iter()
                    .map(|item| {
                        item.as_str().map(str::to_string).with_context(|| {
                            format!("layer exception `{krate}` must contain only strings")
                        })
                    })
                    .collect::<Result<Vec<String>>>()?;
                layer_exceptions.insert(krate.clone(), list);
            }
        }

        Ok(GuardConfig {
            budgets,
            layer_exceptions,
        })
    }

    /// Recorded ceiling for `metric`, if any.
    pub fn budget(&self, metric: Metric) -> Option<usize> {
        self.budgets.get(metric.key()).copied()
    }

    /// Whether `dependency` is recorded debt for `krate`.
    pub fn is_excepted(&self, krate: &str, dependency: &str) -> bool {
        self.layer_exceptions
            .get(krate)
            .is_some_and(|deps| deps.iter().any(|dep| dep == dependency))
    }

    /// Render back to deterministic TOML text.
    pub fn render(&self) -> String {
        let mut text = String::from(HEADER);
        text.push_str("\n[budgets]\n");
        for metric in Metric::ALL {
            let count = self.budget(metric).unwrap_or(0);
            let _ = writeln!(text, "{} = {count}", metric.key());
        }

        text.push_str("\n# Layering debt recorded on 2026-08-05. Entries may be removed as the\n");
        text.push_str("# dependencies are routed through y-service, never added.\n");
        text.push_str("[layers.exceptions]\n");
        for (krate, deps) in &self.layer_exceptions {
            let rendered = deps
                .iter()
                .map(|dep| format!("\"{dep}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(text, "\"{krate}\" = [{rendered}]");
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_budgets_and_exceptions() {
        let config = GuardConfig::parse(
            "[budgets]\npanics = 12\n\n[layers.exceptions]\n\"y-cli\" = [\"y-tools\"]\n",
        )
        .expect("valid config");
        assert_eq!(config.budget(Metric::Panics), Some(12));
        assert_eq!(config.budget(Metric::OversizedFiles), None);
        assert!(config.is_excepted("y-cli", "y-tools"));
        assert!(!config.is_excepted("y-cli", "y-skills"));
        assert!(!config.is_excepted("y-web", "y-tools"));
    }

    #[test]
    fn empty_config_is_valid_and_permits_nothing() {
        let config = GuardConfig::parse("").expect("empty config");
        assert_eq!(config.budget(Metric::Panics), None);
        assert!(!config.is_excepted("y-cli", "y-tools"));
    }

    #[test]
    fn rejects_negative_budget() {
        assert!(GuardConfig::parse("[budgets]\npanics = -1\n").is_err());
    }

    #[test]
    fn rejects_unknown_budget_key() {
        assert!(GuardConfig::parse("[budgets]\nponics = 3\n").is_err());
    }

    #[test]
    fn rejects_non_array_exception() {
        assert!(GuardConfig::parse("[layers.exceptions]\n\"y-cli\" = \"y-tools\"\n").is_err());
    }

    #[test]
    fn render_round_trips() {
        let mut config = GuardConfig::default();
        config.budgets.insert("panics".to_string(), 122);
        config
            .layer_exceptions
            .insert("y-cli".to_string(), vec!["y-tools".to_string()]);

        let reparsed = GuardConfig::parse(&config.render()).expect("rendered config parses");
        assert_eq!(reparsed.budget(Metric::Panics), Some(122));
        assert!(reparsed.is_excepted("y-cli", "y-tools"));
        // Every metric is materialized so a new metric never silently defaults.
        for metric in Metric::ALL {
            assert!(reparsed.budget(metric).is_some(), "{metric} missing");
        }
    }
}
