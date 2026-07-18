use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use y_agent::agent::definition::AgentDefinition;
use y_agent::TrustTier;
use y_storage::workflow_store::WorkflowRow;

use super::manifest::CapabilityResourceKind;
use super::transaction::DeclarativeCapabilityBackend;
use super::validator::ValidatedCapabilityResource;
use crate::agent_management::AgentManagementService;
use crate::container::ServiceContainer;
use crate::workflow_service::{CreateWorkflowRequest, UpdateWorkflowRequest, WorkflowService};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum CapabilityPackOwnerSnapshot {
    Skill {
        previous: Option<PathBuf>,
    },
    Agent {
        previous_file: Option<PathBuf>,
        previous_definition: Option<Box<AgentDefinition>>,
    },
    Workflow {
        previous: Option<Box<WorkflowRow>>,
    },
    ExecutableDeclaration {
        previous: Option<PathBuf>,
    },
}

pub(crate) struct CapabilityPackOwnerBackend<'a> {
    container: &'a ServiceContainer,
    snapshot_root: PathBuf,
}

impl<'a> CapabilityPackOwnerBackend<'a> {
    pub(crate) fn new(container: &'a ServiceContainer) -> Self {
        Self {
            container,
            snapshot_root: container.data_dir.join("capability-packs/snapshots"),
        }
    }

    fn skill_target(&self, resource: &ValidatedCapabilityResource) -> Result<PathBuf, String> {
        self.container
            .skills_dir
            .as_ref()
            .map(|directory| directory.join(&resource.id))
            .ok_or_else(|| "no skills directory configured".to_string())
    }

    fn executable_declaration_target(&self, resource: &ValidatedCapabilityResource) -> PathBuf {
        self.container
            .data_dir
            .join("capability-packs/declarations")
            .join(resource.kind.as_str())
            .join(format!("{}.toml", resource.id))
    }

    async fn agent_target(
        &self,
        resource: &ValidatedCapabilityResource,
    ) -> Result<PathBuf, String> {
        self.container
            .agent_registry
            .lock()
            .await
            .agents_dir()
            .map(|directory| directory.join(format!("{}.toml", resource.id)))
            .ok_or_else(|| "no agents directory configured".to_string())
    }

    fn create_snapshot(&self, source: &Path, label: &str) -> Result<Option<PathBuf>, String> {
        if !source.exists() {
            return Ok(None);
        }
        let directory = self.snapshot_root.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory).map_err(|error| {
            format!(
                "failed to create {label} snapshot directory {}: {error}",
                directory.display()
            )
        })?;
        let snapshot = directory.join("payload");
        copy_entry(source, &snapshot)?;
        sync_tree(&snapshot)?;
        sync_directory(&directory)?;
        Ok(Some(snapshot))
    }

    async fn validate_agent(
        &self,
        resource: &ValidatedCapabilityResource,
    ) -> Result<AgentDefinition, String> {
        let source = read_utf8(&resource.path, "agent")?;
        let registry = self.container.agent_registry.lock().await;
        registry
            .agents_dir()
            .ok_or_else(|| "no agents directory configured".to_string())?;
        let expanded = registry.expand_templates(&source);
        drop(registry);
        let definition = AgentDefinition::from_toml(&expanded)
            .map_err(|error| format!("invalid agent TOML: {error}"))?;
        definition
            .validate()
            .map_err(|error| format!("invalid agent definition: {error}"))?;
        if definition.id != resource.id {
            return Err(format!(
                "agent ID '{}' does not match declared resource ID '{}'",
                definition.id, resource.id
            ));
        }
        Ok(definition)
    }

    fn validate_skill(&self, resource: &ValidatedCapabilityResource) -> Result<(), String> {
        self.skill_target(resource)?;
        let manifest_path = resource.path.join("skill.toml");
        let source = read_utf8(&manifest_path, "skill manifest")?;
        let manifest = y_skills::ManifestParser::new(y_skills::SkillConfig::default())
            .parse(&source)
            .map_err(|error| format!("invalid skill manifest: {error}"))?;
        if manifest.name != resource.id {
            return Err(format!(
                "skill name '{}' does not match declared resource ID '{}'",
                manifest.name, resource.id
            ));
        }
        Ok(())
    }

    fn workflow_source(
        resource: &ValidatedCapabilityResource,
    ) -> Result<(String, &'static str), String> {
        let format = match resource.path.extension().and_then(|value| value.to_str()) {
            Some("toml") => "toml",
            Some("dsl") => "expression_dsl",
            _ => return Err("workflow resources must use .toml or .dsl".to_string()),
        };
        let source = read_utf8(&resource.path, "workflow")?;
        let validation = WorkflowService::validate_definition(&source, format);
        if !validation.valid {
            return Err(format!(
                "invalid workflow definition: {}",
                validation.errors.join("; ")
            ));
        }
        Ok((source, format))
    }

    fn validate_mcp(resource: &ValidatedCapabilityResource) -> Result<(), String> {
        parse_mcp_declaration(&resource.path, &resource.id).map(|_| ())
    }

    fn validate_hook(resource: &ValidatedCapabilityResource) -> Result<(), String> {
        let declaration = parse_hook_declaration(&resource.path)?;
        validate_hook_declaration(&declaration)
    }

    fn validate_lsp(resource: &ValidatedCapabilityResource) -> Result<(), String> {
        parse_lsp_declaration(&resource.path, &resource.id).map(|_| ())
    }

    async fn apply_agent(&self, resource: &ValidatedCapabilityResource) -> Result<(), String> {
        let source = read_utf8(&resource.path, "agent")?;
        let mut definition = self.validate_agent(resource).await?;
        let target = self.agent_target(resource).await?;
        atomic_replace_file(&source.into_bytes(), &target)?;
        definition.trust_tier = TrustTier::UserDefined;
        let mut registry = self.container.agent_registry.lock().await;
        registry
            .register_or_override(definition)
            .map_err(|error| format!("failed to register agent: {error}"))?;
        AgentManagementService::refresh_callable_agents_text(
            &registry,
            &self.container.callable_agents_text,
        )
        .await;
        Ok(())
    }

    async fn restore_agent(
        &self,
        resource: &ValidatedCapabilityResource,
        previous_file: Option<PathBuf>,
        previous_definition: Option<Box<AgentDefinition>>,
    ) -> Result<(), String> {
        let target = self.agent_target(resource).await?;
        if let Some(previous_file) = previous_file {
            let bytes = std::fs::read(&previous_file).map_err(|error| {
                format!(
                    "failed to read agent snapshot {}: {error}",
                    previous_file.display()
                )
            })?;
            atomic_replace_file(&bytes, &target)?;
        } else {
            remove_entry(&target)?;
        }

        let mut registry = self.container.agent_registry.lock().await;
        if let Some(definition) = previous_definition {
            registry
                .register_or_override(*definition)
                .map_err(|error| format!("failed to restore agent definition: {error}"))?;
        } else if registry.get(&resource.id).is_some() {
            registry
                .unregister(&resource.id)
                .map_err(|error| format!("failed to remove restored agent: {error}"))?;
        }
        AgentManagementService::refresh_callable_agents_text(
            &registry,
            &self.container.callable_agents_text,
        )
        .await;
        Ok(())
    }

    async fn apply_workflow(&self, resource: &ValidatedCapabilityResource) -> Result<(), String> {
        let (definition, format) = Self::workflow_source(resource)?;
        if let Some(existing) = self
            .container
            .workflow_store
            .get_by_name(&resource.id)
            .await
            .map_err(|error| error.to_string())?
        {
            WorkflowService::update(
                &self.container.workflow_store,
                &existing.id,
                &UpdateWorkflowRequest {
                    definition: Some(definition),
                    format: Some(format.to_string()),
                    description: None,
                    tags: None,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        } else {
            WorkflowService::create(
                &self.container.workflow_store,
                &CreateWorkflowRequest {
                    name: resource.id.clone(),
                    definition,
                    format: format.to_string(),
                    description: None,
                    tags: None,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    async fn restore_workflow(
        &self,
        resource: &ValidatedCapabilityResource,
        previous: Option<Box<WorkflowRow>>,
    ) -> Result<(), String> {
        let previous = previous.map(|row| *row);
        let current = self
            .container
            .workflow_store
            .get_by_name(&resource.id)
            .await
            .map_err(|error| error.to_string())?;
        match (current, previous) {
            (Some(current), Some(previous)) if current.id == previous.id => {
                let updated = self
                    .container
                    .workflow_store
                    .update(&previous)
                    .await
                    .map_err(|error| error.to_string())?;
                if !updated {
                    return Err(format!(
                        "workflow disappeared during restore: {}",
                        previous.id
                    ));
                }
            }
            (Some(current), Some(previous)) => {
                self.container
                    .workflow_store
                    .delete(&current.id)
                    .await
                    .map_err(|error| error.to_string())?;
                self.container
                    .workflow_store
                    .save(&previous)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            (None, Some(previous)) => {
                self.container
                    .workflow_store
                    .save(&previous)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            (Some(current), None) => {
                self.container
                    .workflow_store
                    .delete(&current.id)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            (None, None) => {}
        }
        Ok(())
    }
}

fn validate_hook_declaration(declaration: &CapabilityPackHookDeclaration) -> Result<(), String> {
    let mut hook_handlers = std::collections::HashMap::new();
    hook_handlers.insert(
        declaration.hook_point.clone(),
        vec![y_hooks::config::HookHandlerGroupConfig {
            matcher: declaration.matcher.clone(),
            timeout_ms: declaration.timeout_ms,
            handlers: declaration.handlers.clone(),
        }],
    );
    let config = y_hooks::HookConfig {
        hook_handlers,
        ..y_hooks::HookConfig::default()
    };
    y_hooks::config::validate_hook_handler_config(&config)
        .map_err(|error| format!("invalid hook declaration: {error}"))?;
    y_hooks::HookHandlerExecutor::from_config(&config)
        .map(|_| ())
        .map_err(|error| format!("invalid hook declaration: {error}"))
}

#[async_trait]
impl DeclarativeCapabilityBackend for CapabilityPackOwnerBackend<'_> {
    type Snapshot = CapabilityPackOwnerSnapshot;

    async fn validate(&self, resource: &ValidatedCapabilityResource) -> Result<(), String> {
        let actual_hash = hash_entry(&resource.path)?;
        if actual_hash != resource.sha256 {
            return Err(format!(
                "resource content changed after staging validation: expected {}, found {actual_hash}",
                resource.sha256
            ));
        }
        match resource.kind {
            CapabilityResourceKind::Skill => self.validate_skill(resource),
            CapabilityResourceKind::Agent => self.validate_agent(resource).await.map(|_| ()),
            CapabilityResourceKind::Workflow => Self::workflow_source(resource).map(|_| ()),
            CapabilityResourceKind::Mcp => Self::validate_mcp(resource),
            CapabilityResourceKind::Hook => Self::validate_hook(resource),
            CapabilityResourceKind::Lsp => Self::validate_lsp(resource),
        }
    }

    async fn current_hash(
        &self,
        resource: &ValidatedCapabilityResource,
    ) -> Result<Option<String>, String> {
        match resource.kind {
            CapabilityResourceKind::Skill => hash_existing(&self.skill_target(resource)?),
            CapabilityResourceKind::Agent => {
                let target = self.agent_target(resource).await?;
                if target.exists() {
                    return hash_existing(&target);
                }
                let definition = self
                    .container
                    .agent_registry
                    .lock()
                    .await
                    .get(&resource.id)
                    .cloned();
                definition
                    .map(|definition| {
                        toml::to_string_pretty(&definition)
                            .map(|source| sha256_bytes(source.as_bytes()))
                            .map_err(|error| format!("failed to serialize current agent: {error}"))
                    })
                    .transpose()
            }
            CapabilityResourceKind::Workflow => self
                .container
                .workflow_store
                .get_by_name(&resource.id)
                .await
                .map_err(|error| error.to_string())
                .map(|row| row.map(|row| sha256_bytes(row.definition.as_bytes()))),
            CapabilityResourceKind::Mcp
            | CapabilityResourceKind::Hook
            | CapabilityResourceKind::Lsp => {
                hash_existing(&self.executable_declaration_target(resource))
            }
        }
    }

    async fn snapshot(
        &self,
        resource: &ValidatedCapabilityResource,
    ) -> Result<Self::Snapshot, String> {
        match resource.kind {
            CapabilityResourceKind::Skill => Ok(CapabilityPackOwnerSnapshot::Skill {
                previous: self.create_snapshot(&self.skill_target(resource)?, "skill")?,
            }),
            CapabilityResourceKind::Agent => {
                let target = self.agent_target(resource).await?;
                let previous_definition = self
                    .container
                    .agent_registry
                    .lock()
                    .await
                    .get(&resource.id)
                    .cloned();
                Ok(CapabilityPackOwnerSnapshot::Agent {
                    previous_file: self.create_snapshot(&target, "agent")?,
                    previous_definition: previous_definition.map(Box::new),
                })
            }
            CapabilityResourceKind::Workflow => Ok(CapabilityPackOwnerSnapshot::Workflow {
                previous: self
                    .container
                    .workflow_store
                    .get_by_name(&resource.id)
                    .await
                    .map_err(|error| error.to_string())?
                    .map(Box::new),
            }),
            CapabilityResourceKind::Mcp
            | CapabilityResourceKind::Hook
            | CapabilityResourceKind::Lsp => {
                Ok(CapabilityPackOwnerSnapshot::ExecutableDeclaration {
                    previous: self.create_snapshot(
                        &self.executable_declaration_target(resource),
                        "executable declaration",
                    )?,
                })
            }
        }
    }

    async fn apply(&self, resource: &ValidatedCapabilityResource) -> Result<(), String> {
        match resource.kind {
            CapabilityResourceKind::Skill => {
                atomic_replace_directory(&resource.path, &self.skill_target(resource)?)?;
                self.container.refresh_skill_search().await;
                Ok(())
            }
            CapabilityResourceKind::Agent => self.apply_agent(resource).await,
            CapabilityResourceKind::Workflow => self.apply_workflow(resource).await,
            CapabilityResourceKind::Mcp
            | CapabilityResourceKind::Hook
            | CapabilityResourceKind::Lsp => atomic_replace_file(
                &std::fs::read(&resource.path).map_err(|error| {
                    format!("failed to read {}: {error}", resource.path.display())
                })?,
                &self.executable_declaration_target(resource),
            ),
        }
    }

    async fn restore(
        &self,
        resource: &ValidatedCapabilityResource,
        snapshot: Self::Snapshot,
    ) -> Result<(), String> {
        match (resource.kind, snapshot) {
            (CapabilityResourceKind::Skill, CapabilityPackOwnerSnapshot::Skill { previous }) => {
                let target = self.skill_target(resource)?;
                if let Some(previous) = previous {
                    atomic_replace_directory(&previous, &target)?;
                } else {
                    remove_entry(&target)?;
                }
                self.container.refresh_skill_search().await;
                Ok(())
            }
            (
                CapabilityResourceKind::Agent,
                CapabilityPackOwnerSnapshot::Agent {
                    previous_file,
                    previous_definition,
                },
            ) => {
                self.restore_agent(resource, previous_file, previous_definition)
                    .await
            }
            (
                CapabilityResourceKind::Workflow,
                CapabilityPackOwnerSnapshot::Workflow { previous },
            ) => self.restore_workflow(resource, previous).await,
            (
                CapabilityResourceKind::Mcp
                | CapabilityResourceKind::Hook
                | CapabilityResourceKind::Lsp,
                CapabilityPackOwnerSnapshot::ExecutableDeclaration { previous },
            ) => {
                let target = self.executable_declaration_target(resource);
                if let Some(previous) = previous {
                    let bytes = std::fs::read(&previous).map_err(|error| {
                        format!("failed to read {}: {error}", previous.display())
                    })?;
                    atomic_replace_file(&bytes, &target)
                } else {
                    remove_entry(&target)
                }
            }
            _ => Err("snapshot kind does not match capability resource kind".to_string()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapabilityPackHookDeclaration {
    pub(crate) hook_point: String,
    #[serde(default = "default_hook_matcher")]
    pub(crate) matcher: String,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) handlers: Vec<y_hooks::config::HandlerConfig>,
}

fn default_hook_matcher() -> String {
    "*".to_string()
}

pub(crate) fn parse_hook_declaration(path: &Path) -> Result<CapabilityPackHookDeclaration, String> {
    let source = read_utf8(path, "hook declaration")?;
    toml::from_str(&source).map_err(|error| format!("invalid hook declaration: {error}"))
}

pub(crate) fn parse_mcp_declaration(
    path: &Path,
    expected_id: &str,
) -> Result<y_tools::McpServerConfig, String> {
    let source = read_utf8(path, "MCP declaration")?;
    let config: y_tools::McpServerConfig =
        toml::from_str(&source).map_err(|error| format!("invalid MCP declaration: {error}"))?;
    if config.name != expected_id {
        return Err(format!(
            "MCP server name '{}' does not match declared resource ID '{expected_id}'",
            config.name
        ));
    }
    match config.transport.as_str() {
        "stdio" if config.command.is_some() => Ok(config),
        "http" if config.url.is_some() => Ok(config),
        "stdio" => Err("stdio MCP declaration requires command".to_string()),
        "http" => Err("http MCP declaration requires url".to_string()),
        other => Err(format!("unsupported MCP transport: {other}")),
    }
}

pub(crate) fn parse_lsp_declaration(
    path: &Path,
    expected_id: &str,
) -> Result<crate::lsp::LspServerConfig, String> {
    let source = read_utf8(path, "LSP declaration")?;
    let config: crate::lsp::LspServerConfig =
        toml::from_str(&source).map_err(|error| format!("invalid LSP declaration: {error}"))?;
    if config.id != expected_id {
        return Err(format!(
            "LSP server ID '{}' does not match declared resource ID '{expected_id}'",
            config.id
        ));
    }
    if config.command.trim().is_empty() {
        return Err("LSP declaration requires command".to_string());
    }
    Ok(config)
}

fn read_utf8(path: &Path, label: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {label} {}: {error}", path.display()))
}

fn hash_existing(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    hash_entry(path).map(Some)
}

pub(super) fn hash_entry(path: &Path) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("symbolic link is not allowed: {}", path.display()));
    }
    if metadata.is_file() {
        return std::fs::read(path)
            .map(|bytes| sha256_bytes(&bytes))
            .map_err(|error| format!("failed to read {}: {error}", path.display()));
    }
    if !metadata.is_dir() {
        return Err(format!("unsupported filesystem entry: {}", path.display()));
    }
    let mut files = Vec::new();
    collect_files(path, path, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"y-agent-capability-pack-directory-v1\0");
    for file in files {
        let relative = file
            .strip_prefix(path)
            .map_err(|error| error.to_string())?
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| "resource path is not valid UTF-8".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("/");
        let mut input = std::fs::File::open(&file)
            .map_err(|error| format!("failed to open {}: {error}", file.display()))?;
        let length = input
            .metadata()
            .map_err(|error| format!("failed to inspect {}: {error}", file.display()))?
            .len();
        hasher.update((relative.len() as u64).to_be_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(length.to_be_bytes());
        let mut buffer = [0_u8; 8192];
        loop {
            let count = input
                .read(&mut buffer)
                .map_err(|error| format!("failed to read {}: {error}", file.display()))?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("symbolic link is not allowed: {}", path.display()));
        }
        if metadata.is_dir() {
            collect_files(root, &path, files)?;
        } else if metadata.is_file() {
            path.strip_prefix(root).map_err(|error| error.to_string())?;
            files.push(path);
        } else {
            return Err(format!("unsupported filesystem entry: {}", path.display()));
        }
    }
    Ok(())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn copy_entry(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(source)
        .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "symbolic link is not allowed: {}",
            source.display()
        ));
    }
    if metadata.is_dir() {
        copy_directory(source, target)
    } else if metadata.is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        std::fs::copy(source, target)
            .map(|_| ())
            .map_err(|error| format!("failed to copy {}: {error}", source.display()))
    } else {
        Err(format!(
            "unsupported filesystem entry: {}",
            source.display()
        ))
    }
}

fn copy_directory(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target)
        .map_err(|error| format!("failed to create {}: {error}", target.display()))?;
    let mut entries = std::fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        copy_entry(&entry.path(), &target.join(entry.file_name()))?;
    }
    Ok(())
}

fn atomic_replace_directory(source: &Path, target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("destination has no parent: {}", target.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let suffix = uuid::Uuid::new_v4();
    let temporary = parent.join(format!(".capability-pack-{suffix}.tmp"));
    let replaced = parent.join(format!(".capability-pack-{suffix}.replaced"));
    copy_directory(source, &temporary)?;
    sync_tree(&temporary)?;
    let had_target = target.exists();
    if had_target {
        std::fs::rename(target, &replaced)
            .map_err(|error| format!("failed to stage existing destination: {error}"))?;
    }
    if let Err(error) = std::fs::rename(&temporary, target) {
        if had_target {
            let _ = std::fs::rename(&replaced, target);
        }
        return Err(format!("failed to commit directory replacement: {error}"));
    }
    sync_directory(parent)?;
    if had_target {
        remove_entry(&replaced)?;
    }
    Ok(())
}

fn atomic_replace_file(bytes: &[u8], target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("destination has no parent: {}", target.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let suffix = uuid::Uuid::new_v4();
    let temporary = parent.join(format!(".capability-pack-{suffix}.tmp"));
    let replaced = parent.join(format!(".capability-pack-{suffix}.replaced"));
    let mut file = std::fs::File::create(&temporary)
        .map_err(|error| format!("failed to create {}: {error}", temporary.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", temporary.display()))?;
    let had_target = target.exists();
    if had_target {
        std::fs::rename(target, &replaced)
            .map_err(|error| format!("failed to stage existing destination: {error}"))?;
    }
    if let Err(error) = std::fs::rename(&temporary, target) {
        if had_target {
            let _ = std::fs::rename(&replaced, target);
        }
        return Err(format!("failed to commit file replacement: {error}"));
    }
    sync_directory(parent)?;
    if had_target {
        remove_entry(&replaced)?;
    }
    Ok(())
}

fn remove_entry(path: &Path) -> Result<(), String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path)
            .map_err(|error| format!("failed to remove {}: {error}", path.display()))
    } else {
        std::fs::remove_file(path)
            .map_err(|error| format!("failed to remove {}: {error}", path.display()))
    }
}

fn sync_tree(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if metadata.is_file() {
        return std::fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("failed to sync {}: {error}", path.display()));
    }
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        sync_tree(&entry.path())?;
    }
    sync_directory(path)
}

fn sync_directory(path: &Path) -> Result<(), String> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("failed to sync directory {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::capability_pack::{
        CapabilityPackInstallOptions, CapabilityPackProvenance, CapabilityPackSourceKind,
        CapabilityPackTransactionJournal, CapabilityPackTransactionState,
        CapabilityPackTransactionStatus, CapabilityResourceKind, DeclarativeCapabilityBackend,
        DurableCapabilityPackInstaller, ValidatedCapabilityPack, ValidatedCapabilityResource,
    };
    use crate::{ServiceConfig, ServiceContainer};

    async fn setup() -> (tempfile::TempDir, ServiceConfig, ServiceContainer) {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path();
        let config_dir = root.join("config");
        std::fs::create_dir_all(config_dir.join("prompts")).expect("prompts dir");
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: root.join("state.db").display().to_string(),
            pool_size: 1,
            wal_enabled: true,
            transcript_dir: root.join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        config.prompts_dir = Some(config_dir.join("prompts"));
        config.skills_dir = Some(config_dir.join("skills"));
        let container = ServiceContainer::from_config(&config)
            .await
            .expect("service container");
        (temp, config, container)
    }

    fn resource(
        kind: CapabilityResourceKind,
        id: &str,
        path: impl Into<PathBuf>,
    ) -> ValidatedCapabilityResource {
        let path = path.into();
        ValidatedCapabilityResource {
            kind,
            id: id.to_string(),
            sha256: hash_entry(&path).expect("resource hash"),
            path,
        }
    }

    fn write_agent(path: &Path, id: &str, prompt: &str) {
        std::fs::write(
            path,
            format!(
                r#"id = "{id}"
name = "{id}"
description = "Capability pack test agent"
mode = "general"
trust_tier = "user_defined"
system_prompt = "{prompt}"
"#,
            ),
        )
        .expect("agent source");
    }

    fn write_skill(path: &Path, name: &str, content: &str) {
        std::fs::create_dir_all(path).expect("skill dir");
        std::fs::write(
            path.join("skill.toml"),
            format!(
                r#"name = "{name}"
description = "Capability pack test skill"
root_content = "{content}"
"#,
            ),
        )
        .expect("skill manifest");
        std::fs::write(path.join("root.md"), content).expect("skill root");
    }

    #[tokio::test]
    async fn semantic_preflight_rejects_agent_identity_mismatch() {
        let (temp, _config, container) = setup().await;
        let source = temp.path().join("agent.toml");
        write_agent(&source, "actual-agent", "Review carefully.");
        let backend = CapabilityPackOwnerBackend::new(&container);

        let error = backend
            .validate(&resource(
                CapabilityResourceKind::Agent,
                "declared-agent",
                source,
            ))
            .await
            .expect_err("identity mismatch");

        assert!(error.contains("does not match declared resource ID"));
    }

    #[tokio::test]
    async fn semantic_preflight_rejects_content_changed_after_staging_validation() {
        let (temp, _config, container) = setup().await;
        let source = temp.path().join("agent.toml");
        write_agent(&source, "reviewer", "Original prompt.");
        let resource = resource(CapabilityResourceKind::Agent, "reviewer", &source);
        write_agent(&source, "reviewer", "Changed after validation.");
        let backend = CapabilityPackOwnerBackend::new(&container);

        let error = backend
            .validate(&resource)
            .await
            .expect_err("content drift");

        assert!(error.contains("content changed after staging validation"));
    }

    #[tokio::test]
    async fn executable_resources_are_staged_without_runtime_activation() {
        let (temp, _config, container) = setup().await;
        let pack_root = temp.path().join("executable-pack");
        std::fs::create_dir_all(&pack_root).expect("pack root");
        let mcp = pack_root.join("pack-mcp.toml");
        let hook = pack_root.join("audit-hook.toml");
        let lsp = pack_root.join("pack-lsp.toml");
        std::fs::write(
            &mcp,
            r#"name = "pack-mcp"
transport = "stdio"
command = "must-not-start"
"#,
        )
        .expect("MCP declaration");
        std::fs::write(
            &hook,
            r#"hook_point = "pre_tool_execute"

[[handlers]]
type = "command"
command = "/bin/true"
"#,
        )
        .expect("hook declaration");
        std::fs::write(
            &lsp,
            r#"id = "pack-lsp"
command = "must-not-start"
language_id = "rust"
extensions = ["rs"]
root_markers = ["Cargo.toml"]
"#,
        )
        .expect("LSP declaration");
        let resources = vec![
            resource(CapabilityResourceKind::Mcp, "pack-mcp", mcp),
            resource(CapabilityResourceKind::Hook, "audit-hook", hook),
            resource(CapabilityResourceKind::Lsp, "pack-lsp", lsp),
        ];
        let backend = CapabilityPackOwnerBackend::new(&container);
        let mut snapshots = Vec::new();

        for resource in &resources {
            backend
                .validate(resource)
                .await
                .expect("semantic preflight");
            let snapshot = backend.snapshot(resource).await.expect("snapshot");
            backend.apply(resource).await.expect("inactive staging");
            assert!(backend.executable_declaration_target(resource).is_file());
            snapshots.push((resource, snapshot));
        }

        assert_eq!(container.mcp_manager.connected_count().await, 0);
        assert!(container
            .tool_registry
            .get_all_definitions()
            .await
            .iter()
            .all(|definition| !definition.name.as_str().starts_with("mcp_pack-mcp_")));

        for (resource, snapshot) in snapshots.into_iter().rev() {
            backend
                .restore(resource, snapshot)
                .await
                .expect("remove inactive staging");
            assert!(!backend.executable_declaration_target(resource).exists());
        }
    }

    #[tokio::test]
    async fn owner_backend_applies_and_restores_all_declarative_kinds() {
        let (temp, _config, container) = setup().await;
        let pack_root = temp.path().join("pack");
        std::fs::create_dir_all(&pack_root).expect("pack root");
        let skill_source = pack_root.join("review-rust");
        let agent_source = pack_root.join("reviewer.toml");
        let workflow_source = pack_root.join("release-flow.dsl");
        write_skill(&skill_source, "review-rust", "Review ownership carefully.");
        write_agent(&agent_source, "reviewer", "Review Rust changes.");
        std::fs::write(&workflow_source, "prepare >> verify").expect("workflow source");
        let resources = vec![
            resource(CapabilityResourceKind::Skill, "review-rust", skill_source),
            resource(CapabilityResourceKind::Agent, "reviewer", agent_source),
            resource(
                CapabilityResourceKind::Workflow,
                "release-flow",
                workflow_source,
            ),
        ];
        let backend = CapabilityPackOwnerBackend::new(&container);

        let mut snapshots = Vec::new();
        for resource in &resources {
            backend
                .validate(resource)
                .await
                .expect("semantic preflight");
            let snapshot = backend.snapshot(resource).await.expect("snapshot");
            backend.apply(resource).await.expect("apply");
            snapshots.push((resource, snapshot));
        }

        assert!(container
            .skills_dir
            .as_ref()
            .expect("skills dir")
            .join("review-rust/root.md")
            .is_file());
        assert!(container
            .agent_registry
            .lock()
            .await
            .get("reviewer")
            .is_some());
        assert!(container
            .workflow_store
            .get_by_name("release-flow")
            .await
            .expect("workflow lookup")
            .is_some());

        for (resource, snapshot) in snapshots.into_iter().rev() {
            backend.restore(resource, snapshot).await.expect("restore");
        }

        assert!(!container
            .skills_dir
            .as_ref()
            .expect("skills dir")
            .join("review-rust")
            .exists());
        assert!(container
            .agent_registry
            .lock()
            .await
            .get("reviewer")
            .is_none());
        assert!(container
            .workflow_store
            .get_by_name("release-flow")
            .await
            .expect("workflow lookup")
            .is_none());
    }

    #[tokio::test]
    async fn durable_installer_commits_real_owner_resources_and_snapshots() {
        let (temp, _config, container) = setup().await;
        let pack_root = temp.path().join("durable-pack");
        std::fs::create_dir_all(&pack_root).expect("pack root");
        let skill_source = pack_root.join("review-rust");
        let agent_source = pack_root.join("reviewer.toml");
        let workflow_source = pack_root.join("release-flow.dsl");
        write_skill(&skill_source, "review-rust", "Review ownership carefully.");
        write_agent(&agent_source, "reviewer", "Review Rust changes.");
        std::fs::write(&workflow_source, "prepare >> verify").expect("workflow source");
        let pack = ValidatedCapabilityPack {
            schema_version: 1,
            id: "durable-pack".into(),
            version: "1.0.0".into(),
            description: None,
            provenance: CapabilityPackProvenance {
                source_kind: CapabilityPackSourceKind::LocalDirectory,
                pack_root: pack_root.clone(),
                manifest_path: pack_root.join("capability-pack.toml"),
                manifest_sha256: "f".repeat(64),
            },
            resources: vec![
                resource(CapabilityResourceKind::Skill, "review-rust", skill_source),
                resource(CapabilityResourceKind::Agent, "reviewer", agent_source),
                resource(
                    CapabilityResourceKind::Workflow,
                    "release-flow",
                    workflow_source,
                ),
            ],
        };
        let journal = CapabilityPackTransactionJournal::new(
            container.data_dir.join("capability-packs/transactions"),
        );
        let backend = CapabilityPackOwnerBackend::new(&container);

        let receipt = DurableCapabilityPackInstaller::install(
            &backend,
            &journal,
            &pack,
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("durable install");

        assert_eq!(receipt.applied.len(), 3);
        let records = journal.load_all().expect("journal records");
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].status,
            CapabilityPackTransactionStatus::Committed
        );
        assert!(records[0]
            .resources
            .iter()
            .all(|resource| resource.snapshot.is_some()));
    }

    #[tokio::test]
    async fn owner_backend_restores_replaced_skill_agent_and_workflow() {
        let (temp, _config, container) = setup().await;
        let live_skill = container
            .skills_dir
            .as_ref()
            .expect("skills dir")
            .join("review-rust");
        write_skill(&live_skill, "review-rust", "Original skill.");
        let old_agent_source = temp.path().join("old-agent.toml");
        write_agent(&old_agent_source, "reviewer", "Original agent.");
        container
            .save_agent(
                "reviewer",
                &std::fs::read_to_string(&old_agent_source).expect("old agent"),
            )
            .await
            .expect("save old agent");
        WorkflowService::create(
            &container.workflow_store,
            &CreateWorkflowRequest {
                name: "release-flow".into(),
                definition: "prepare >> verify".into(),
                format: "expression_dsl".into(),
                description: Some("Original workflow".into()),
                tags: Some("original".into()),
            },
        )
        .await
        .expect("create old workflow");

        let pack_root = temp.path().join("replacement-pack");
        std::fs::create_dir_all(&pack_root).expect("pack root");
        let skill_source = pack_root.join("review-rust");
        let agent_source = pack_root.join("reviewer.toml");
        let workflow_source = pack_root.join("release-flow.dsl");
        write_skill(&skill_source, "review-rust", "Replacement skill.");
        write_agent(&agent_source, "reviewer", "Replacement agent.");
        std::fs::write(&workflow_source, "prepare >> release >> verify").expect("workflow source");
        let resources = vec![
            resource(CapabilityResourceKind::Skill, "review-rust", skill_source),
            resource(CapabilityResourceKind::Agent, "reviewer", agent_source),
            resource(
                CapabilityResourceKind::Workflow,
                "release-flow",
                workflow_source,
            ),
        ];
        let backend = CapabilityPackOwnerBackend::new(&container);
        let mut snapshots = Vec::new();
        for resource in &resources {
            backend
                .validate(resource)
                .await
                .expect("semantic preflight");
            let snapshot = backend.snapshot(resource).await.expect("snapshot");
            backend.apply(resource).await.expect("apply replacement");
            snapshots.push((resource, snapshot));
        }
        for (resource, snapshot) in snapshots.into_iter().rev() {
            backend
                .restore(resource, snapshot)
                .await
                .expect("restore replacement");
        }

        assert_eq!(
            std::fs::read_to_string(live_skill.join("root.md")).expect("skill root"),
            "Original skill."
        );
        assert_eq!(
            container
                .agent_registry
                .lock()
                .await
                .get("reviewer")
                .expect("restored agent")
                .system_prompt,
            "Original agent."
        );
        let workflow = container
            .workflow_store
            .get_by_name("release-flow")
            .await
            .expect("workflow lookup")
            .expect("restored workflow");
        assert_eq!(workflow.definition, "prepare >> verify");
        assert_eq!(workflow.description.as_deref(), Some("Original workflow"));
        assert_eq!(workflow.tags, r#"["original"]"#);
    }

    #[tokio::test]
    async fn service_startup_recovers_interrupted_owner_transaction() {
        let (temp, config, container) = setup().await;
        let live_skill = container
            .skills_dir
            .as_ref()
            .expect("skills dir")
            .join("review-rust");
        write_skill(&live_skill, "review-rust", "Original instructions.");
        let source = temp.path().join("pack/review-rust");
        write_skill(&source, "review-rust", "Replacement instructions.");
        let resource = resource(CapabilityResourceKind::Skill, "review-rust", source);
        let pack = ValidatedCapabilityPack {
            schema_version: 1,
            id: "test-pack".into(),
            version: "1.0.0".into(),
            description: None,
            provenance: CapabilityPackProvenance {
                source_kind: CapabilityPackSourceKind::LocalDirectory,
                pack_root: temp.path().join("pack"),
                manifest_path: temp.path().join("pack/capability-pack.toml"),
                manifest_sha256: "f".repeat(64),
            },
            resources: vec![resource.clone()],
        };
        let backend = CapabilityPackOwnerBackend::new(&container);
        let journal = CapabilityPackTransactionJournal::new(
            container.data_dir.join("capability-packs/transactions"),
        );
        let mut record = journal.begin(&pack).expect("begin transaction");
        let snapshot = backend.snapshot(&resource).await.expect("snapshot");
        backend.apply(&resource).await.expect("simulated apply");
        record.status = CapabilityPackTransactionStatus::Applying;
        record.resources[0].snapshot =
            Some(serde_json::to_value(snapshot).expect("snapshot serialization"));
        record.resources[0].state = CapabilityPackTransactionState::Applied;
        journal.save(&record).expect("persist interrupted state");
        drop(container);

        let _reopened = ServiceContainer::from_config(&config)
            .await
            .expect("startup recovery");

        assert_eq!(
            std::fs::read_to_string(live_skill.join("root.md")).expect("restored skill"),
            "Original instructions."
        );
        assert_eq!(
            journal.load(&record.id).expect("recovered record").status,
            CapabilityPackTransactionStatus::RolledBack
        );
    }
}
