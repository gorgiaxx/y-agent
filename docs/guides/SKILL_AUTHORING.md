# Skill Authoring Guide

Skills are higher-level capabilities composed of tools, prompts, and workflows. They enable agents to learn and apply reusable patterns.

## Skill Structure

A skill is defined by a **manifest file** (`skill.toml`) and optional supporting files:

```
skills/
└── code_review/
    ├── skill.toml          # Manifest: name, description, triggers
    ├── prompt.md           # System prompt template
    ├── workflow.toml        # Optional DAG workflow definition
    └── tools/              # Optional skill-specific tools
        └── lint_checker.rs
```

## Manifest Format

```toml
# skill.toml

[skill]
name = "code_review"
version = "1.0.0"
description = "Reviews code for quality, bugs, and style issues"
author = "you"

# When this skill should be activated
[triggers]
keywords = ["review", "code quality", "lint", "refactor"]
file_patterns = ["*.rs", "*.py", "*.ts"]
explicit = false  # false = can be auto-activated by keyword match

# Required tools
[dependencies]
tools = ["read_file", "write_file", "shell_exec"]

# Skill parameters
[parameters]
language = { type = "string", required = true }
style_guide = { type = "string", required = false, default = "standard" }
severity = { type = "string", enum = ["info", "warning", "error"], default = "warning" }
```

## Prompt Templates

Prompts support variable interpolation:

```markdown
<!-- prompt.md -->

You are a code reviewer specializing in {{language}}.

Review the following code according to the {{style_guide}} style guide.
Report issues at {{severity}} level or above.

Focus on:
1. Logic errors and potential bugs
2. Performance concerns
3. Security vulnerabilities
4. Code style and readability
```

## Skill Discovery

Skills are discovered through the `SkillRegistry`:

1. **Directory scan** — `~/.config/y-agent/skills/` and `./skills/`
2. **Manifest validation** — Schema validation of `skill.toml`
3. **Dependency check** — Verify required tools are available
4. **Index building** — Create searchable skill index

## Activation

Skills can be activated:

- **Explicitly** — User requests a skill by name: "use the code_review skill"
- **Automatically** — Keyword matching against the skill's trigger rules
- **Programmatically** — Agent invokes a skill during workflow execution

## Workflow Integration

Skills can define multi-step workflows:

```toml
# workflow.toml

[[steps]]
name = "read_source"
tool = "read_file"
args = { path = "{{file_path}}" }

[[steps]]
name = "analyze"
depends_on = ["read_source"]
tool = "llm_call"
prompt = "Analyze this code for issues: {{read_source.output}}"

[[steps]]
name = "format_report"
depends_on = ["analyze"]
tool = "llm_call"
prompt = "Format the analysis into a structured report"
```

## Testing Skills

```rust
use y_skill::registry::SkillRegistry;

#[tokio::test]
async fn test_skill_discovery() {
    let registry = SkillRegistry::from_directory("./test_skills").await.unwrap();
    let skills = registry.list().await;
    assert!(!skills.is_empty());
}

#[tokio::test]
async fn test_skill_search() {
    let registry = SkillRegistry::from_directory("./test_skills").await.unwrap();
    let results = registry.search("code review").await.unwrap();
    assert!(results.iter().any(|s| s.name == "code_review"));
}
```

## Best Practices

1. **Keep skills focused** — One skill per domain (code review, documentation, testing)
2. **Declare dependencies** — List all required tools in the manifest
3. **Use templates** — Parameterize prompts for flexibility
4. **Version your skills** — Semantic versioning for compatibility
5. **Test triggers** — Ensure keywords don't overlap with other skills
