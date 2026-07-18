use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::manifest::{
    CapabilityPackManifest, CapabilityResourceDeclaration, CapabilityResourceKind,
};

const MANIFEST_FILE: &str = "capability-pack.toml";
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Machine-readable validation failure categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPackIssueCode {
    PackRootInvalid,
    ManifestMissing,
    ManifestSymlink,
    ManifestPathEscape,
    ManifestReadError,
    ManifestParseError,
    UnsupportedSchemaVersion,
    InvalidPackId,
    InvalidPackVersion,
    EmptyResources,
    InvalidResourceId,
    DuplicateResourceIdentity,
    InvalidHash,
    AbsoluteResourcePath,
    ParentTraversal,
    MissingResource,
    SymlinkNotAllowed,
    ResourcePathEscape,
    ResourceTypeMismatch,
    UnsupportedFilesystemEntry,
    ResourceReadError,
    HashMismatch,
}

/// One deterministic validation issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackIssue {
    pub code: CapabilityPackIssueCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_kind: Option<CapabilityResourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

/// Origin type retained for later install and activation stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPackSourceKind {
    LocalDirectory,
}

impl CapabilityPackSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalDirectory => "local_directory",
        }
    }
}

/// Canonical source provenance produced by successful manifest parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackProvenance {
    pub source_kind: CapabilityPackSourceKind,
    pub pack_root: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest_sha256: String,
}

/// One path- and hash-verified resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedCapabilityResource {
    pub kind: CapabilityResourceKind,
    pub id: String,
    pub path: PathBuf,
    pub sha256: String,
}

/// Parsed pack identity plus resources that passed every validation stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedCapabilityPack {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub description: Option<String>,
    pub provenance: CapabilityPackProvenance,
    pub resources: Vec<ValidatedCapabilityResource>,
}

/// Side-effect-free result of validating one local pack directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackValidationReport {
    pub valid: bool,
    pub pack: Option<ValidatedCapabilityPack>,
    pub issues: Vec<CapabilityPackIssue>,
}

impl CapabilityPackValidationReport {
    pub fn has_issue(&self, code: CapabilityPackIssueCode) -> bool {
        self.issues.iter().any(|issue| issue.code == code)
    }
}

pub struct CapabilityPackValidator;

impl CapabilityPackValidator {
    pub fn validate(pack_root: &Path) -> CapabilityPackValidationReport {
        let mut issues = Vec::new();
        let canonical_root = match std::fs::canonicalize(pack_root) {
            Ok(path) if path.is_dir() => path,
            Ok(path) => {
                issues.push(issue(
                    CapabilityPackIssueCode::PackRootInvalid,
                    format!("pack root is not a directory: {}", path.display()),
                    None,
                ));
                return report(None, issues);
            }
            Err(error) => {
                issues.push(issue(
                    CapabilityPackIssueCode::PackRootInvalid,
                    format!(
                        "cannot canonicalize pack root {}: {error}",
                        pack_root.display()
                    ),
                    Some(pack_root.to_path_buf()),
                ));
                return report(None, issues);
            }
        };
        let manifest_path = canonical_root.join(MANIFEST_FILE);
        let manifest_metadata = match std::fs::symlink_metadata(&manifest_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                issues.push(issue(
                    CapabilityPackIssueCode::ManifestMissing,
                    format!("cannot read {}: {error}", manifest_path.display()),
                    Some(manifest_path),
                ));
                return report(None, issues);
            }
        };
        if manifest_metadata.file_type().is_symlink() {
            issues.push(issue(
                CapabilityPackIssueCode::ManifestSymlink,
                "capability-pack.toml must not be a symbolic link".to_string(),
                Some(manifest_path),
            ));
            return report(None, issues);
        }
        let canonical_manifest = match std::fs::canonicalize(&manifest_path) {
            Ok(path) if path.starts_with(&canonical_root) => path,
            Ok(path) => {
                issues.push(issue(
                    CapabilityPackIssueCode::ManifestPathEscape,
                    format!("manifest escapes pack root: {}", path.display()),
                    Some(path),
                ));
                return report(None, issues);
            }
            Err(error) => {
                issues.push(issue(
                    CapabilityPackIssueCode::ManifestReadError,
                    format!("cannot canonicalize manifest: {error}"),
                    Some(manifest_path),
                ));
                return report(None, issues);
            }
        };
        let manifest_bytes = match std::fs::read(&canonical_manifest) {
            Ok(bytes) => bytes,
            Err(error) => {
                issues.push(issue(
                    CapabilityPackIssueCode::ManifestReadError,
                    format!("cannot read manifest: {error}"),
                    Some(canonical_manifest),
                ));
                return report(None, issues);
            }
        };
        let manifest: CapabilityPackManifest =
            match toml::from_str(&String::from_utf8_lossy(&manifest_bytes)) {
                Ok(manifest) => manifest,
                Err(error) => {
                    issues.push(issue(
                        CapabilityPackIssueCode::ManifestParseError,
                        error.to_string(),
                        Some(canonical_manifest),
                    ));
                    return report(None, issues);
                }
            };
        let provenance = CapabilityPackProvenance {
            source_kind: CapabilityPackSourceKind::LocalDirectory,
            pack_root: canonical_root.clone(),
            manifest_path: canonical_manifest,
            manifest_sha256: sha256_bytes(&manifest_bytes),
        };

        validate_manifest_metadata(&manifest, &mut issues);
        let mut seen = BTreeSet::new();
        let mut resources = Vec::new();
        for declaration in &manifest.resources {
            let identity = (declaration.kind, declaration.id.clone());
            if !seen.insert(identity) {
                issues.push(resource_issue(
                    CapabilityPackIssueCode::DuplicateResourceIdentity,
                    declaration,
                    format!(
                        "duplicate resource identity {}:{}",
                        declaration.kind.as_str(),
                        declaration.id
                    ),
                    Some(PathBuf::from(&declaration.path)),
                ));
            }
            if let Some(resource) = validate_resource(declaration, &canonical_root, &mut issues) {
                resources.push(resource);
            }
        }
        resources.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.id.cmp(&right.id))
        });

        let pack = ValidatedCapabilityPack {
            schema_version: manifest.pack.schema_version,
            id: manifest.pack.id,
            version: manifest.pack.version,
            description: manifest.pack.description,
            provenance,
            resources,
        };
        report(Some(pack), issues)
    }
}

fn validate_manifest_metadata(
    manifest: &CapabilityPackManifest,
    issues: &mut Vec<CapabilityPackIssue>,
) {
    if manifest.pack.schema_version != SUPPORTED_SCHEMA_VERSION {
        issues.push(issue(
            CapabilityPackIssueCode::UnsupportedSchemaVersion,
            format!(
                "unsupported schema version {}; expected {SUPPORTED_SCHEMA_VERSION}",
                manifest.pack.schema_version
            ),
            None,
        ));
    }
    if !valid_id(&manifest.pack.id) {
        issues.push(issue(
            CapabilityPackIssueCode::InvalidPackId,
            format!("invalid pack id '{}'", manifest.pack.id),
            None,
        ));
    }
    if semver::Version::parse(&manifest.pack.version).is_err() {
        issues.push(issue(
            CapabilityPackIssueCode::InvalidPackVersion,
            format!("invalid semantic version '{}'", manifest.pack.version),
            None,
        ));
    }
    if manifest.resources.is_empty() {
        issues.push(issue(
            CapabilityPackIssueCode::EmptyResources,
            "capability pack declares no resources".to_string(),
            None,
        ));
    }
}

fn validate_resource(
    declaration: &CapabilityResourceDeclaration,
    pack_root: &Path,
    issues: &mut Vec<CapabilityPackIssue>,
) -> Option<ValidatedCapabilityResource> {
    let issue_count = issues.len();
    if !valid_id(&declaration.id) {
        issues.push(resource_issue(
            CapabilityPackIssueCode::InvalidResourceId,
            declaration,
            format!("invalid resource id '{}'", declaration.id),
            Some(PathBuf::from(&declaration.path)),
        ));
    }
    if !valid_sha256(&declaration.sha256) {
        issues.push(resource_issue(
            CapabilityPackIssueCode::InvalidHash,
            declaration,
            "sha256 must contain exactly 64 lowercase hexadecimal characters".to_string(),
            Some(PathBuf::from(&declaration.path)),
        ));
    }
    let relative = Path::new(&declaration.path);
    if looks_absolute(relative, &declaration.path) {
        issues.push(resource_issue(
            CapabilityPackIssueCode::AbsoluteResourcePath,
            declaration,
            "resource path must be relative to the pack root".to_string(),
            Some(relative.to_path_buf()),
        ));
        return None;
    }
    if relative
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        issues.push(resource_issue(
            CapabilityPackIssueCode::ParentTraversal,
            declaration,
            "resource path must not contain parent traversal".to_string(),
            Some(relative.to_path_buf()),
        ));
        return None;
    }
    let joined = pack_root.join(relative);
    if let Some(symlink_path) = first_symlink_component(pack_root, relative) {
        issues.push(resource_issue(
            CapabilityPackIssueCode::SymlinkNotAllowed,
            declaration,
            "symbolic links are not allowed in capability pack resource paths".to_string(),
            Some(symlink_path),
        ));
        return None;
    }
    let metadata = match std::fs::symlink_metadata(&joined) {
        Ok(metadata) => metadata,
        Err(error) => {
            issues.push(resource_issue(
                CapabilityPackIssueCode::MissingResource,
                declaration,
                format!("cannot inspect resource: {error}"),
                Some(joined),
            ));
            return None;
        }
    };
    if metadata.file_type().is_symlink() {
        issues.push(resource_issue(
            CapabilityPackIssueCode::SymlinkNotAllowed,
            declaration,
            "symbolic links are not allowed in capability packs".to_string(),
            Some(joined),
        ));
        return None;
    }
    let canonical = match std::fs::canonicalize(&joined) {
        Ok(path) if path.starts_with(pack_root) => path,
        Ok(path) => {
            issues.push(resource_issue(
                CapabilityPackIssueCode::ResourcePathEscape,
                declaration,
                "resource canonical path escapes the pack root".to_string(),
                Some(path),
            ));
            return None;
        }
        Err(error) => {
            issues.push(resource_issue(
                CapabilityPackIssueCode::MissingResource,
                declaration,
                format!("cannot canonicalize resource: {error}"),
                Some(joined),
            ));
            return None;
        }
    };
    let expects_directory = declaration.kind.expects_directory();
    if expects_directory != metadata.is_dir() {
        issues.push(resource_issue(
            CapabilityPackIssueCode::ResourceTypeMismatch,
            declaration,
            if expects_directory {
                "skill resource must be a directory".to_string()
            } else {
                "non-skill resource must be a regular file".to_string()
            },
            Some(canonical),
        ));
        return None;
    }
    if !metadata.is_file() && !metadata.is_dir() {
        issues.push(resource_issue(
            CapabilityPackIssueCode::UnsupportedFilesystemEntry,
            declaration,
            "resource is neither a regular file nor a directory".to_string(),
            Some(canonical),
        ));
        return None;
    }

    let actual_hash = hash_resource(&canonical, declaration, issues)?;
    if valid_sha256(&declaration.sha256) && actual_hash != declaration.sha256 {
        issues.push(resource_issue(
            CapabilityPackIssueCode::HashMismatch,
            declaration,
            format!(
                "resource hash mismatch: expected {}, actual {actual_hash}",
                declaration.sha256
            ),
            Some(canonical.clone()),
        ));
    }
    if issues.len() != issue_count {
        return None;
    }
    Some(ValidatedCapabilityResource {
        kind: declaration.kind,
        id: declaration.id.clone(),
        path: canonical,
        sha256: actual_hash,
    })
}

fn hash_resource(
    path: &Path,
    declaration: &CapabilityResourceDeclaration,
    issues: &mut Vec<CapabilityPackIssue>,
) -> Option<String> {
    if path.is_file() {
        return match std::fs::read(path) {
            Ok(bytes) => Some(sha256_bytes(&bytes)),
            Err(error) => {
                issues.push(resource_issue(
                    CapabilityPackIssueCode::ResourceReadError,
                    declaration,
                    format!("cannot read resource: {error}"),
                    Some(path.to_path_buf()),
                ));
                None
            }
        };
    }
    let mut files = Vec::new();
    if !collect_directory_files(path, declaration, issues, &mut files) {
        return None;
    }
    files.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"y-agent-capability-pack-directory-v1\0");
    for file in files {
        let relative = match file.strip_prefix(path) {
            Ok(relative) => relative,
            Err(error) => {
                issues.push(resource_issue(
                    CapabilityPackIssueCode::ResourcePathEscape,
                    declaration,
                    error.to_string(),
                    Some(file),
                ));
                return None;
            }
        };
        let Some(relative) = normalized_relative_path(relative) else {
            issues.push(resource_issue(
                CapabilityPackIssueCode::UnsupportedFilesystemEntry,
                declaration,
                "resource path is not valid UTF-8".to_string(),
                Some(file),
            ));
            return None;
        };
        let mut input = match std::fs::File::open(&file) {
            Ok(input) => input,
            Err(error) => {
                issues.push(resource_issue(
                    CapabilityPackIssueCode::ResourceReadError,
                    declaration,
                    format!("cannot open resource file: {error}"),
                    Some(file),
                ));
                return None;
            }
        };
        let file_len = match input.metadata() {
            Ok(metadata) => metadata.len(),
            Err(error) => {
                issues.push(resource_issue(
                    CapabilityPackIssueCode::ResourceReadError,
                    declaration,
                    format!("cannot read resource metadata: {error}"),
                    Some(file),
                ));
                return None;
            }
        };
        hasher.update((relative.len() as u64).to_be_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(file_len.to_be_bytes());
        let mut buffer = [0_u8; 8192];
        loop {
            match input.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => hasher.update(&buffer[..count]),
                Err(error) => {
                    issues.push(resource_issue(
                        CapabilityPackIssueCode::ResourceReadError,
                        declaration,
                        format!("cannot read resource file: {error}"),
                        Some(file),
                    ));
                    return None;
                }
            }
        }
    }
    Some(hex::encode(hasher.finalize()))
}

fn collect_directory_files(
    directory: &Path,
    declaration: &CapabilityResourceDeclaration,
    issues: &mut Vec<CapabilityPackIssue>,
    files: &mut Vec<PathBuf>,
) -> bool {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            issues.push(resource_issue(
                CapabilityPackIssueCode::ResourceReadError,
                declaration,
                format!("cannot read resource directory: {error}"),
                Some(directory.to_path_buf()),
            ));
            return false;
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => {
                issues.push(resource_issue(
                    CapabilityPackIssueCode::ResourceReadError,
                    declaration,
                    format!("cannot read resource directory entry: {error}"),
                    Some(directory.to_path_buf()),
                ));
                return false;
            }
        }
    }
    paths.sort();
    for path in paths {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                issues.push(resource_issue(
                    CapabilityPackIssueCode::ResourceReadError,
                    declaration,
                    format!("cannot inspect resource entry: {error}"),
                    Some(path),
                ));
                return false;
            }
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            issues.push(resource_issue(
                CapabilityPackIssueCode::SymlinkNotAllowed,
                declaration,
                "symbolic links are not allowed in capability packs".to_string(),
                Some(path),
            ));
            return false;
        }
        if file_type.is_dir() {
            if !collect_directory_files(&path, declaration, issues, files) {
                return false;
            }
        } else if file_type.is_file() {
            files.push(path);
        } else {
            issues.push(unsupported_entry_issue(declaration, path));
            return false;
        }
    }
    true
}

fn unsupported_entry_issue(
    declaration: &CapabilityResourceDeclaration,
    path: PathBuf,
) -> CapabilityPackIssue {
    resource_issue(
        CapabilityPackIssueCode::UnsupportedFilesystemEntry,
        declaration,
        "resource contains an unsupported filesystem entry".to_string(),
        Some(path),
    )
}

fn normalized_relative_path(path: &Path) -> Option<String> {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;
    Some(components.join("/"))
}

fn looks_absolute(path: &Path, raw: &str) -> bool {
    path.is_absolute()
        || raw.starts_with("\\\\")
        || raw.as_bytes().get(1) == Some(&b':')
            && raw.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
}

fn first_symlink_component(pack_root: &Path, relative: &Path) -> Option<PathBuf> {
    let mut current = pack_root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(segment) => current.push(segment),
            Component::CurDir => continue,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Some(current),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

fn valid_id(value: &str) -> bool {
    (3..=64).contains(&value.len())
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn report(
    pack: Option<ValidatedCapabilityPack>,
    mut issues: Vec<CapabilityPackIssue>,
) -> CapabilityPackValidationReport {
    issues.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.resource_kind.cmp(&right.resource_kind))
            .then_with(|| left.resource_id.cmp(&right.resource_id))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.message.cmp(&right.message))
    });
    CapabilityPackValidationReport {
        valid: issues.is_empty(),
        pack,
        issues,
    }
}

fn issue(
    code: CapabilityPackIssueCode,
    message: String,
    path: Option<PathBuf>,
) -> CapabilityPackIssue {
    CapabilityPackIssue {
        code,
        message,
        resource_kind: None,
        resource_id: None,
        path,
    }
}

fn resource_issue(
    code: CapabilityPackIssueCode,
    declaration: &CapabilityResourceDeclaration,
    message: String,
    path: Option<PathBuf>,
) -> CapabilityPackIssue {
    CapabilityPackIssue {
        code,
        message,
        resource_kind: Some(declaration.kind),
        resource_id: Some(declaration.id.clone()),
        path,
    }
}
