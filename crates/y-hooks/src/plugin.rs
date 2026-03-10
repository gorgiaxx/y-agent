//! Plugin loader skeleton.
//!
//! Full dynamic plugin loading is deferred to Phase 4.
//! This module provides the interface and placeholder implementation.

/// Plugin metadata loaded from a plugin manifest.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// Unique plugin name.
    pub name: String,
    /// Plugin version.
    pub version: String,
    /// Path to the shared library (for dynamic loading).
    pub lib_path: Option<String>,
    /// Middleware chain registrations this plugin provides.
    pub middleware: Vec<String>,
    /// Hook points this plugin listens to.
    pub hooks: Vec<String>,
}

/// Plugin loader — skeleton for Phase 4.
///
/// In Phase 4, this will support loading plugins from shared libraries
/// or WASM modules. For now, it provides the interface only.
pub struct PluginLoader;

impl PluginLoader {
    /// Create a new plugin loader.
    pub fn new() -> Self {
        Self
    }

    /// Load a plugin from its manifest.
    ///
    /// **Phase 4 TODO**: Implement actual dynamic loading.
    pub fn load(&self, _manifest: &PluginManifest) -> Result<(), String> {
        Err("plugin loading not yet implemented (Phase 4)".into())
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PluginLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginLoader").finish()
    }
}
