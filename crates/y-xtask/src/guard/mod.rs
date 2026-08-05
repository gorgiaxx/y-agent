//! Architecture and quality guards.
//!
//! `AGENTS.md` and `docs/guides/ARCHITECTURE.md` state rules that nothing used to
//! verify. Every unverified rule has to be re-derived and re-checked by hand on
//! each change; a guard converts that recurring cost into a one-time cost.

pub mod budgets;
pub mod config;
pub mod layers;
pub mod memory;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};

use crate::guard::budgets::{Metric, Verdict, OVERSIZED_FILE_LOC};
use crate::guard::config::GuardConfig;

/// Name of the ratchet baseline file, relative to the repository root.
const CONFIG_FILE: &str = "guards.toml";

/// Directories never scanned.
const SKIPPED_DIRS: &[&str] = &["target", "node_modules", "dist", ".git"];

/// Guard subcommands.
#[derive(Debug, Subcommand)]
pub enum GuardCommand {
    /// Verify workspace layering rules.
    Architecture,
    /// Verify ratcheting quality budgets.
    Budgets(BudgetArgs),
    /// Verify documented memory ceilings against their source declarations.
    Memory,
    /// Run every guard.
    All(BudgetArgs),
}

/// Options shared by the budget-aware subcommands.
#[derive(Debug, Args)]
pub struct BudgetArgs {
    /// Rewrite `guards.toml` from the current measurements instead of failing.
    #[arg(long)]
    pub update: bool,
}

/// Execute a guard subcommand.
pub fn run(command: &GuardCommand, root: &Path) -> Result<()> {
    match command {
        GuardCommand::Architecture => check_architecture(root, false),
        GuardCommand::Budgets(args) => check_budgets(root, args.update),
        GuardCommand::Memory => memory::check(root),
        GuardCommand::All(args) => {
            let architecture = check_architecture(root, args.update);
            let budgets = check_budgets(root, args.update);
            let memory = memory::check(root);
            architecture.and(budgets).and(memory)
        }
    }
}

/// Load `guards.toml`, tolerating its absence during initial seeding.
fn load_config(root: &Path) -> Result<GuardConfig> {
    let path = root.join(CONFIG_FILE);
    if !path.exists() {
        return Ok(GuardConfig::default());
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    GuardConfig::parse(&text)
}

/// Persist `config` back to `guards.toml`.
fn store_config(root: &Path, config: &GuardConfig) -> Result<()> {
    let path = root.join(CONFIG_FILE);
    fs::write(&path, config.render()).with_context(|| format!("write {}", path.display()))
}

// --- architecture ---------------------------------------------------------

/// Workspace-internal dependency graph, read from `cargo metadata`.
fn workspace_graph(root: &Path) -> Result<Vec<(String, Vec<String>)>> {
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .context("run cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse cargo metadata output")?;
    let packages = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .context("cargo metadata has no packages array")?;

    let names: Vec<String> = packages
        .iter()
        .filter_map(|package| package.get("name")?.as_str().map(str::to_string))
        .collect();

    let mut graph = Vec::with_capacity(packages.len());
    for package in packages {
        let Some(name) = package.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let deps = package
            .get("dependencies")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("name")?.as_str().map(str::to_string))
                    .filter(|dep| names.contains(dep))
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        graph.push((name.to_string(), deps));
    }
    graph.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(graph)
}

/// Check layering, honouring (and optionally regenerating) recorded debt.
fn check_architecture(root: &Path, update: bool) -> Result<()> {
    let graph = workspace_graph(root)?;
    let violations = layers::check_workspace(&graph);
    let mut config = load_config(root)?;

    if update {
        let mut exceptions: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for violation in &violations {
            exceptions
                .entry(violation.krate.clone())
                .or_default()
                .push(violation.dependency.clone());
        }
        config.layer_exceptions = exceptions;
        store_config(root, &config)?;
        println!("architecture: recorded {} exception(s)", violations.len());
        return Ok(());
    }

    let unexpected: Vec<_> = violations
        .iter()
        .filter(|violation| !config.is_excepted(&violation.krate, &violation.dependency))
        .collect();

    let resolved: Vec<(String, String)> = config
        .layer_exceptions
        .iter()
        .flat_map(|(krate, deps)| deps.iter().map(move |dep| (krate.clone(), dep.clone())))
        .filter(|(krate, dep)| {
            !violations
                .iter()
                .any(|violation| &violation.krate == krate && &violation.dependency == dep)
        })
        .collect();

    let unassigned = layers::unassigned(&graph);

    for violation in &unexpected {
        println!("  FAIL {}", violation.describe());
    }
    for (krate, dep) in &resolved {
        println!("  STALE exception {krate} -> {dep} is no longer a violation");
    }
    for krate in &unassigned {
        println!("  FAIL {krate} has no layer assignment in guard/layers.rs");
    }

    if unexpected.is_empty() && resolved.is_empty() && unassigned.is_empty() {
        println!(
            "architecture: ok ({} crate(s), {} recorded exception(s))",
            graph.len(),
            violations.len()
        );
        return Ok(());
    }
    bail!(
        "architecture guard failed: {} new violation(s), {} stale exception(s), \
         {} unassigned crate(s). Route the dependency through y-service, assign \
         the crate a layer, or run `guard all --update` after removing a \
         resolved exception.",
        unexpected.len(),
        resolved.len(),
        unassigned.len()
    )
}

// --- budgets --------------------------------------------------------------

/// A file that participates in the budgets, with its scanned production text.
struct ScannedFile {
    path: PathBuf,
    lines: usize,
    production: String,
}

/// Whether `path` is a Rust file holding production code.
fn is_production_rust(path: &Path) -> bool {
    if path.extension().is_none_or(|extension| extension != "rs") {
        return false;
    }
    if path
        .components()
        .any(|component| component.as_os_str() == "tests")
    {
        return false;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    !(name.ends_with("_test.rs") || name.ends_with("_tests.rs") || name == "tests.rs")
}

/// Whether `path` is a frontend source file subject to the suppression budget.
fn is_frontend_source(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    if !matches!(extension, "ts" | "tsx") {
        return false;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if name.contains(".test.") || name.contains(".spec.") {
        return false;
    }
    !path
        .components()
        .any(|component| component.as_os_str() == "__tests__")
}

/// Per-metric totals paired with the worst offenders for each metric.
type Measurements = (BTreeMap<Metric, usize>, BTreeMap<Metric, Vec<String>>);

/// Recursively collect files under `dir` accepted by `accept`.
fn collect_files(dir: &Path, accept: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) -> Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("read entry in {}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if SKIPPED_DIRS.contains(&name.as_ref()) {
                continue;
            }
            collect_files(&path, accept, out)?;
        } else if accept(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// Read and pre-slice every production Rust file under `crates/`.
fn scan_rust(root: &Path) -> Result<Vec<ScannedFile>> {
    let mut paths = Vec::new();
    collect_files(&root.join("crates"), &is_production_rust, &mut paths)?;
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let source =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            Ok(ScannedFile {
                lines: source.lines().count(),
                production: budgets::production_slice(&source).to_string(),
                path,
            })
        })
        .collect()
}

/// Measure every metric across the workspace.
fn measure(root: &Path) -> Result<Measurements> {
    let rust = scan_rust(root)?;
    let mut counts: BTreeMap<Metric, usize> = BTreeMap::new();
    let mut offenders: BTreeMap<Metric, Vec<(usize, String)>> = BTreeMap::new();

    for file in &rust {
        for metric in Metric::ALL {
            let count = if metric == Metric::OversizedFiles {
                usize::from(file.lines > OVERSIZED_FILE_LOC)
            } else {
                budgets::count_rust(metric, &file.production)
            };
            if count == 0 {
                continue;
            }
            *counts.entry(metric).or_default() += count;
            let weight = if metric == Metric::OversizedFiles {
                file.lines
            } else {
                count
            };
            offenders
                .entry(metric)
                .or_default()
                .push((weight, file.path.display().to_string()));
        }
    }

    let mut frontend = Vec::new();
    collect_files(
        &root.join("crates/y-gui/src"),
        &is_frontend_source,
        &mut frontend,
    )?;
    frontend.sort();
    for path in frontend {
        let source =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let count = budgets::count_typescript_suppressions(&source);
        if count == 0 {
            continue;
        }
        *counts.entry(Metric::LintSuppressions).or_default() += count;
        offenders
            .entry(Metric::LintSuppressions)
            .or_default()
            .push((count, path.display().to_string()));
    }

    for metric in Metric::ALL {
        counts.entry(metric).or_default();
    }

    let top = offenders
        .into_iter()
        .map(|(metric, mut items)| {
            items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            items.truncate(5);
            let rendered = items
                .into_iter()
                .map(|(weight, path)| format!("{path} ({weight})"))
                .collect();
            (metric, rendered)
        })
        .collect();

    Ok((counts, top))
}

/// Check every quality budget, or reseed the baselines.
fn check_budgets(root: &Path, update: bool) -> Result<()> {
    let (counts, offenders) = measure(root)?;
    let mut config = load_config(root)?;

    if update {
        config.budgets = counts
            .iter()
            .map(|(metric, count)| (metric.key().to_string(), *count))
            .collect();
        store_config(root, &config)?;
        for (metric, count) in &counts {
            println!("budget {metric}: recorded {count}");
        }
        return Ok(());
    }

    let mut failures = Vec::new();
    for metric in Metric::ALL {
        let count = counts.get(&metric).copied().unwrap_or_default();
        let Some(budget) = config.budget(metric) else {
            failures.push(format!(
                "{metric}: no budget recorded; run `guard budgets --update`"
            ));
            continue;
        };
        match budgets::verdict(count, budget) {
            Verdict::Met => println!("budget {metric}: ok ({count})"),
            Verdict::Exceeded { excess } => {
                failures.push(format!(
                    "{metric}: {count} exceeds budget {budget} by {excess} -- {}",
                    metric.rationale()
                ));
                for offender in offenders.get(&metric).into_iter().flatten() {
                    println!("    {offender}");
                }
            }
            Verdict::Stale { tighten_to } => failures.push(format!(
                "{metric}: improved to {tighten_to} (budget {budget}); lower the budget to lock it in"
            )),
        }
    }

    if failures.is_empty() {
        return Ok(());
    }
    for failure in &failures {
        println!("  FAIL {failure}");
    }
    bail!("budget guard failed: {} metric(s)", failures.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_rust_excludes_test_files_and_dirs() {
        assert!(is_production_rust(Path::new("crates/y-core/src/lib.rs")));
        assert!(!is_production_rust(Path::new("crates/y-core/tests/api.rs")));
        assert!(!is_production_rust(Path::new(
            "crates/y-core/src/foo_test.rs"
        )));
        assert!(!is_production_rust(Path::new(
            "crates/y-core/src/foo_tests.rs"
        )));
        assert!(!is_production_rust(Path::new("crates/y-core/src/tests.rs")));
        assert!(!is_production_rust(Path::new("crates/y-core/src/lib.md")));
    }

    #[test]
    fn frontend_source_excludes_specs_and_test_dirs() {
        assert!(is_frontend_source(Path::new("crates/y-gui/src/App.tsx")));
        assert!(!is_frontend_source(Path::new(
            "crates/y-gui/src/App.test.tsx"
        )));
        assert!(!is_frontend_source(Path::new(
            "crates/y-gui/src/__tests__/App.tsx"
        )));
        assert!(!is_frontend_source(Path::new("crates/y-gui/src/App.css")));
    }
}
