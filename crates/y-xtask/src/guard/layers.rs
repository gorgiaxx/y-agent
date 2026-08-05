//! Workspace layering rules.
//!
//! `docs/guides/ARCHITECTURE.md` declares that dependencies point inward toward
//! `y-core`, and `AGENTS.md` §2.9 declares that presentation crates are thin I/O
//! wrappers over `y-service`. Both rules were previously documentation-only.
//! This module turns them into a mechanical check over `cargo metadata`.

/// Ordered layer table. Index is the layer rank: a crate may depend only on
/// crates whose rank is less than or equal to its own.
const LAYERS: &[(&str, &[&str])] = &[
    ("Core", &["y-core"]),
    (
        "Infrastructure",
        &[
            "y-provider",
            "y-session",
            "y-context",
            "y-storage",
            "y-knowledge",
            "y-diagnostics",
        ],
    ),
    (
        "Middleware",
        &["y-hooks", "y-guardrails", "y-prompt", "y-mcp"],
    ),
    (
        "Capabilities",
        &[
            "y-tools",
            "y-skills",
            "y-runtime",
            "y-scheduler",
            "y-browser",
            "y-journal",
        ],
    ),
    ("Orchestration", &["y-agent", "y-bot"]),
    ("Service", &["y-service"]),
    ("Presentation", &["y-cli", "y-web", "y-gui"]),
];

/// Crates that carry no layer: test helpers and build tooling. They are ignored
/// as dependents and permitted as dependencies from anywhere.
const UNLAYERED: &[&str] = &["y-test-utils", "y-xtask"];

/// Dependencies a presentation crate may take beyond its sibling presentation
/// crates. This is the mechanical form of `AGENTS.md` §2.9.
const PRESENTATION_ALLOWED: &[&str] = &["y-service", "y-core"];

/// Rank of `crate_name` in the layer table, or `None` when unlayered.
fn rank(crate_name: &str) -> Option<usize> {
    LAYERS
        .iter()
        .position(|(_, members)| members.contains(&crate_name))
}

/// Human-readable layer name for `crate_name`.
fn layer_name(crate_name: &str) -> &'static str {
    rank(crate_name).map_or("Unlayered", |index| LAYERS[index].0)
}

/// A single illegal dependency edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The crate declaring the dependency.
    pub krate: String,
    /// The workspace crate being depended on.
    pub dependency: String,
    /// Why the edge is illegal.
    pub reason: String,
}

impl Violation {
    /// Render as a single diagnostic line.
    pub fn describe(&self) -> String {
        format!(
            "{} ({}) -> {} ({}): {}",
            self.krate,
            layer_name(&self.krate),
            self.dependency,
            layer_name(&self.dependency),
            self.reason
        )
    }
}

/// Check one crate's workspace-internal dependencies against the layer rules.
///
/// `deps` must contain only workspace crates; external crates are not layered.
pub fn check_crate(krate: &str, deps: &[String]) -> Vec<Violation> {
    if UNLAYERED.contains(&krate) {
        return Vec::new();
    }
    let Some(own_rank) = rank(krate) else {
        return Vec::new();
    };
    let is_presentation = own_rank == LAYERS.len() - 1;

    let mut violations = Vec::new();
    for dep in deps {
        let dep = dep.as_str();
        if dep == krate || UNLAYERED.contains(&dep) {
            continue;
        }
        let Some(dep_rank) = rank(dep) else {
            continue;
        };

        if dep_rank > own_rank {
            violations.push(Violation {
                krate: krate.to_string(),
                dependency: dep.to_string(),
                reason: "dependency points outward; dependencies must point inward toward y-core"
                    .to_string(),
            });
            continue;
        }

        if is_presentation && dep_rank != own_rank && !PRESENTATION_ALLOWED.contains(&dep) {
            violations.push(Violation {
                krate: krate.to_string(),
                dependency: dep.to_string(),
                reason: "presentation crates may only reach the service layer (AGENTS.md 2.9)"
                    .to_string(),
            });
        }
    }
    violations
}

/// Check every crate in the workspace graph.
///
/// `graph` maps a workspace crate name to its workspace-internal dependencies.
pub fn check_workspace(graph: &[(String, Vec<String>)]) -> Vec<Violation> {
    let mut violations: Vec<Violation> = graph
        .iter()
        .flat_map(|(krate, deps)| check_crate(krate, deps))
        .collect();
    violations.sort_by(|a, b| (&a.krate, &a.dependency).cmp(&(&b.krate, &b.dependency)));
    violations
}

/// Workspace crates that the layer table does not classify.
///
/// A crate added to the workspace without a layer assignment would otherwise be
/// silently exempt from every rule, so the guard reports it as a failure.
pub fn unassigned(graph: &[(String, Vec<String>)]) -> Vec<String> {
    graph
        .iter()
        .map(|(krate, _)| krate.clone())
        .filter(|krate| rank(krate).is_none() && !UNLAYERED.contains(&krate.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn inward_dependency_is_allowed() {
        assert!(check_crate("y-service", &deps(&["y-core", "y-provider", "y-tools"])).is_empty());
    }

    #[test]
    fn same_layer_dependency_is_allowed() {
        assert!(check_crate("y-diagnostics", &deps(&["y-storage"])).is_empty());
        assert!(check_crate("y-tools", &deps(&["y-browser"])).is_empty());
    }

    #[test]
    fn outward_dependency_is_rejected() {
        let found = check_crate("y-context", &deps(&["y-core", "y-tools"]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].dependency, "y-tools");
        assert!(found[0].reason.contains("inward"));
    }

    #[test]
    fn presentation_may_not_bypass_service() {
        let found = check_crate("y-cli", &deps(&["y-service", "y-core", "y-tools"]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].dependency, "y-tools");
        assert!(found[0].reason.contains("2.9"));
    }

    #[test]
    fn presentation_may_depend_on_sibling_presentation() {
        assert!(check_crate("y-cli", &deps(&["y-web", "y-service"])).is_empty());
    }

    #[test]
    fn unlayered_crates_are_exempt_and_permitted() {
        assert!(check_crate("y-xtask", &deps(&["y-cli"])).is_empty());
        assert!(check_crate("y-session", &deps(&["y-test-utils"])).is_empty());
    }

    #[test]
    fn external_dependencies_are_ignored() {
        assert!(check_crate("y-core", &deps(&["tokio", "serde"])).is_empty());
    }

    #[test]
    fn workspace_check_is_sorted_and_aggregated() {
        let graph = vec![
            ("y-context".to_string(), deps(&["y-tools", "y-prompt"])),
            ("y-cli".to_string(), deps(&["y-agent"])),
        ];
        let found = check_workspace(&graph);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].krate, "y-cli");
        assert_eq!(found[1].dependency, "y-prompt");
        assert_eq!(found[2].dependency, "y-tools");
    }

    #[test]
    fn describe_includes_both_layer_names() {
        let found = check_crate("y-context", &deps(&["y-skills"]));
        let text = found[0].describe();
        assert!(text.contains("y-context (Infrastructure)"), "{text}");
        assert!(text.contains("y-skills (Capabilities)"), "{text}");
    }

    #[test]
    fn unassigned_reports_only_unclassified_crates() {
        let graph = vec![
            ("y-core".to_string(), Vec::new()),
            ("y-test-utils".to_string(), Vec::new()),
            ("y-brand-new".to_string(), Vec::new()),
        ];
        assert_eq!(unassigned(&graph), vec!["y-brand-new".to_string()]);
    }
}
