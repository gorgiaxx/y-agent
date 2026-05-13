//! Runtime template variables for expanding `{{VAR_NAME}}` in agent prompts.
//!
//! Two expansion tiers:
//!
//! | Variable | Expansion Time | Description |
//! |----------|---------------|-------------|
//! | `{{OS}}` | Load + Dispatch | Operating system |
//! | `{{ARCH}}` | Load + Dispatch | CPU architecture |
//! | `{{DATETIME}}` | Dispatch only | Current UTC datetime |
//! | `{{WORKSPACE}}` | Dispatch only | Session workspace path |
//! | `{{ENV:NAME}}` | Dispatch only | OS environment variable |

use chrono::Utc;

/// Runtime template variables for expanding `{{VAR_NAME}}` placeholders.
#[derive(Debug, Clone, Default)]
pub struct RuntimeTemplateVars {
    vars: Vec<(String, String)>,
}

impl RuntimeTemplateVars {
    /// Build template variables from the current runtime state.
    pub fn from_runtime(workspace: Option<&str>) -> Self {
        let now = Utc::now();
        let datetime = format!("{} UTC", now.format("%Y-%m-%d %H:%M:%S"));

        let vars = vec![
            ("{{DATETIME}}".to_string(), datetime),
            ("{{OS}}".to_string(), std::env::consts::OS.to_string()),
            ("{{ARCH}}".to_string(), std::env::consts::ARCH.to_string()),
            (
                "{{WORKSPACE}}".to_string(),
                workspace.unwrap_or("").to_string(),
            ),
        ];

        Self { vars }
    }

    /// Return only the static variables (OS, ARCH) for load-time expansion.
    pub fn static_vars() -> Vec<(String, String)> {
        vec![
            ("{{OS}}".to_string(), std::env::consts::OS.to_string()),
            ("{{ARCH}}".to_string(), std::env::consts::ARCH.to_string()),
        ]
    }

    /// Add or update a custom template variable.
    pub fn add_var(&mut self, key: String, value: String) {
        if let Some(entry) = self.vars.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = value;
        } else {
            self.vars.push((key, value));
        }
    }

    /// Expand template variables in the given content.
    ///
    /// First replaces all registered `{{KEY}}` vars, then resolves any
    /// remaining `{{ENV:VAR_NAME}}` patterns via `std::env::var()`.
    pub fn expand(&self, content: &str) -> String {
        if !Self::content_has_templates(content) {
            return content.to_string();
        }

        let mut result = content.to_string();

        for (key, val) in &self.vars {
            if result.contains(key) {
                result = result.replace(key, val);
            }
        }

        Self::expand_env_vars(&result)
    }

    /// Check whether the content contains any `{{` template markers.
    pub fn content_has_templates(content: &str) -> bool {
        content.contains("{{")
    }

    /// Resolve `{{ENV:VAR_NAME}}` patterns against OS environment variables.
    fn expand_env_vars(content: &str) -> String {
        const PREFIX: &str = "{{ENV:";
        const SUFFIX: &str = "}}";

        let mut result = String::with_capacity(content.len());
        let mut remaining = content;

        while let Some(start) = remaining.find(PREFIX) {
            result.push_str(&remaining[..start]);
            let after_prefix = start + PREFIX.len();
            let Some(end) = remaining[after_prefix..].find(SUFFIX) else {
                result.push_str(&remaining[start..]);
                return result;
            };
            let var_name = &remaining[after_prefix..after_prefix + end];
            let value = std::env::var(var_name).unwrap_or_default();
            result.push_str(&value);

            let after_suffix = after_prefix + end + SUFFIX.len();
            remaining = &remaining[after_suffix..];
        }

        result.push_str(remaining);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_runtime_expands_all_vars() {
        let vars = RuntimeTemplateVars::from_runtime(Some("/home/user/project"));
        let input = "Time: {{DATETIME}}, OS: {{OS}}, Arch: {{ARCH}}, WS: {{WORKSPACE}}";
        let result = vars.expand(input);

        assert!(!result.contains("{{DATETIME}}"));
        assert!(result.contains("UTC"));
        assert!(result.contains(std::env::consts::OS));
        assert!(result.contains(std::env::consts::ARCH));
        assert!(result.contains("/home/user/project"));
    }

    #[test]
    fn test_workspace_none_expands_empty() {
        let vars = RuntimeTemplateVars::from_runtime(None);
        let result = vars.expand("ws={{WORKSPACE}}end");
        assert_eq!(result, "ws=end");
    }

    #[test]
    fn test_env_var_expansion() {
        std::env::set_var("Y_TEMPLATE_TEST_VAR", "hello_template");
        let vars = RuntimeTemplateVars::from_runtime(None);
        let result = vars.expand("val={{ENV:Y_TEMPLATE_TEST_VAR}}");
        assert_eq!(result, "val=hello_template");
        std::env::remove_var("Y_TEMPLATE_TEST_VAR");
    }

    #[test]
    fn test_env_var_missing_expands_empty() {
        let vars = RuntimeTemplateVars::from_runtime(None);
        let result = vars.expand("val={{ENV:Y_NONEXISTENT_VAR_12345}}");
        assert_eq!(result, "val=");
    }

    #[test]
    fn test_no_templates_unchanged() {
        let vars = RuntimeTemplateVars::from_runtime(None);
        let input = "no templates here";
        let result = vars.expand(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_content_has_templates() {
        assert!(RuntimeTemplateVars::content_has_templates(
            "hello {{DATETIME}}"
        ));
        assert!(!RuntimeTemplateVars::content_has_templates("no templates"));
    }

    #[test]
    fn test_static_vars() {
        let svars = RuntimeTemplateVars::static_vars();
        assert_eq!(svars.len(), 2);

        let keys: Vec<&str> = svars.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"{{OS}}"));
        assert!(keys.contains(&"{{ARCH}}"));

        let os_val = svars.iter().find(|(k, _)| k == "{{OS}}").unwrap();
        assert_eq!(os_val.1, std::env::consts::OS);
    }

    #[test]
    fn test_add_var() {
        let mut vars = RuntimeTemplateVars::default();
        vars.add_var("{{CUSTOM}}".to_string(), "custom_value".to_string());
        let result = vars.expand("key={{CUSTOM}}");
        assert_eq!(result, "key=custom_value");
    }

    #[test]
    fn test_add_var_overwrites() {
        let mut vars = RuntimeTemplateVars::default();
        vars.add_var("{{X}}".to_string(), "old".to_string());
        vars.add_var("{{X}}".to_string(), "new".to_string());
        let result = vars.expand("{{X}}");
        assert_eq!(result, "new");
    }

    #[test]
    fn test_datetime_format() {
        let vars = RuntimeTemplateVars::from_runtime(None);
        let result = vars.expand("{{DATETIME}}");
        // Format: "YYYY-MM-DD HH:MM:SS UTC"
        assert!(result.ends_with(" UTC"));
        assert_eq!(result.len(), "2025-01-15 14:30:00 UTC".len());
    }

    #[test]
    fn test_multiple_env_vars() {
        std::env::set_var("Y_TPL_A", "aaa");
        std::env::set_var("Y_TPL_B", "bbb");
        let vars = RuntimeTemplateVars::from_runtime(None);
        let result = vars.expand("{{ENV:Y_TPL_A}}-{{ENV:Y_TPL_B}}");
        assert_eq!(result, "aaa-bbb");
        std::env::remove_var("Y_TPL_A");
        std::env::remove_var("Y_TPL_B");
    }

    #[test]
    fn test_env_var_value_is_not_recursively_expanded() {
        std::env::set_var("Y_TPL_OUTER", "{{ENV:Y_TPL_INNER}}");
        std::env::set_var("Y_TPL_INNER", "expanded");

        let vars = RuntimeTemplateVars::from_runtime(None);
        let result = vars.expand("{{ENV:Y_TPL_OUTER}}");

        assert_eq!(result, "{{ENV:Y_TPL_INNER}}");
        std::env::remove_var("Y_TPL_OUTER");
        std::env::remove_var("Y_TPL_INNER");
    }
}
