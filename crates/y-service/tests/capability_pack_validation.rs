#![cfg(feature = "capability_packs")]

use std::path::Path;

use sha2::{Digest, Sha256};
use y_service::capability_pack::{CapabilityPackIssueCode, CapabilityPackValidator};

fn write_manifest(root: &Path, content: &str) {
    std::fs::write(root.join("capability-pack.toml"), content).expect("manifest");
}

fn file_sha256(path: &Path) -> String {
    hex::encode(Sha256::digest(std::fs::read(path).expect("resource")))
}

fn directory_sha256(root: &Path, relative_files: &[&str]) -> String {
    let mut files = relative_files.to_vec();
    files.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(b"y-agent-capability-pack-directory-v1\0");
    for relative in files {
        let bytes = std::fs::read(root.join(relative)).expect("directory resource");
        hasher.update((relative.len() as u64).to_be_bytes());
        hasher.update(relative.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    hex::encode(hasher.finalize())
}

#[test]
fn rejects_unsupported_schema_versions_and_duplicate_resource_identities() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(temp.path().join("one.toml"), "name = 'one'").expect("one");
    std::fs::write(temp.path().join("two.toml"), "name = 'two'").expect("two");
    write_manifest(
        temp.path(),
        r#"
[pack]
schema_version = 99
id = "example-pack"
version = "1.0.0"

[[resources]]
kind = "agent"
id = "reviewer"
path = "one.toml"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[[resources]]
kind = "agent"
id = "reviewer"
path = "two.toml"
sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
"#,
    );

    let report = CapabilityPackValidator::validate(temp.path());

    assert!(!report.valid);
    assert!(report.has_issue(CapabilityPackIssueCode::UnsupportedSchemaVersion));
    assert!(report.has_issue(CapabilityPackIssueCode::DuplicateResourceIdentity));
}

#[test]
fn rejects_absolute_paths_and_parent_traversal() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    write_manifest(
        temp.path(),
        r#"
[pack]
schema_version = 1
id = "unsafe-paths"
version = "1.0.0"

[[resources]]
kind = "agent"
id = "absolute-agent"
path = "/tmp/agent.toml"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[[resources]]
kind = "workflow"
id = "parent-workflow"
path = "../workflow.toml"
sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
"#,
    );

    let report = CapabilityPackValidator::validate(temp.path());

    assert!(report.has_issue(CapabilityPackIssueCode::AbsoluteResourcePath));
    assert!(report.has_issue(CapabilityPackIssueCode::ParentTraversal));
}

#[cfg(unix)]
#[test]
fn rejects_symlink_resources_even_when_the_target_exists() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::TempDir::new().expect("tempdir");
    let outside = tempfile::NamedTempFile::new().expect("outside");
    symlink(outside.path(), temp.path().join("linked.toml")).expect("symlink");
    write_manifest(
        temp.path(),
        r#"
[pack]
schema_version = 1
id = "linked-pack"
version = "1.0.0"

[[resources]]
kind = "agent"
id = "linked-agent"
path = "linked.toml"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#,
    );

    let report = CapabilityPackValidator::validate(temp.path());

    assert!(report.has_issue(CapabilityPackIssueCode::SymlinkNotAllowed));
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_parent_components_even_when_they_resolve_inside_the_pack() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::TempDir::new().expect("tempdir");
    let actual = temp.path().join("actual");
    std::fs::create_dir_all(&actual).expect("actual dir");
    std::fs::write(actual.join("agent.toml"), "name = 'reviewer'").expect("agent");
    symlink(&actual, temp.path().join("alias")).expect("alias symlink");
    write_manifest(
        temp.path(),
        r#"
[pack]
schema_version = 1
id = "parent-link-pack"
version = "1.0.0"

[[resources]]
kind = "agent"
id = "reviewer"
path = "alias/agent.toml"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#,
    );

    let report = CapabilityPackValidator::validate(temp.path());

    assert!(report.has_issue(CapabilityPackIssueCode::SymlinkNotAllowed));
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_nested_inside_directory_resources() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::TempDir::new().expect("tempdir");
    let skill = temp.path().join("skills/review");
    std::fs::create_dir_all(&skill).expect("skill dir");
    std::fs::write(skill.join("root.md"), "Review carefully.").expect("root");
    let outside = tempfile::NamedTempFile::new().expect("outside");
    symlink(outside.path(), skill.join("details.md")).expect("nested symlink");
    write_manifest(
        temp.path(),
        r#"
[pack]
schema_version = 1
id = "nested-link-pack"
version = "1.0.0"

[[resources]]
kind = "skill"
id = "review"
path = "skills/review"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#,
    );

    let report = CapabilityPackValidator::validate(temp.path());

    assert!(report.has_issue(CapabilityPackIssueCode::SymlinkNotAllowed));
}

#[test]
fn rejects_hash_mismatches_before_returning_validated_resources() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(temp.path().join("agent.toml"), "name = 'reviewer'").expect("agent");
    write_manifest(
        temp.path(),
        r#"
[pack]
schema_version = 1
id = "hash-pack"
version = "1.0.0"

[[resources]]
kind = "agent"
id = "reviewer"
path = "agent.toml"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#,
    );

    let report = CapabilityPackValidator::validate(temp.path());

    assert!(report.has_issue(CapabilityPackIssueCode::HashMismatch));
    assert!(report.pack.expect("parsed pack").resources.is_empty());
}

#[test]
fn valid_reports_are_deterministic_and_retain_local_provenance() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let resource = temp.path().join("agent.toml");
    std::fs::write(&resource, "name = 'reviewer'").expect("agent");
    let hash = file_sha256(&resource);
    write_manifest(
        temp.path(),
        &format!(
            r#"
[pack]
schema_version = 1
id = "valid-pack"
version = "1.2.3"
description = "A deterministic local pack"

[[resources]]
kind = "agent"
id = "reviewer"
path = "agent.toml"
sha256 = "{hash}"
"#
        ),
    );

    let first = CapabilityPackValidator::validate(temp.path());
    let second = CapabilityPackValidator::validate(temp.path());

    assert!(first.valid, "{:#?}", first.issues);
    assert_eq!(first, second);
    let pack = first.pack.expect("validated pack");
    assert_eq!(pack.id, "valid-pack");
    assert_eq!(pack.version, "1.2.3");
    assert_eq!(pack.resources.len(), 1);
    assert_eq!(pack.provenance.source_kind.as_str(), "local_directory");
    assert_eq!(
        pack.provenance.pack_root,
        std::fs::canonicalize(temp.path()).expect("canonical root")
    );
}

#[test]
fn directory_hashes_include_sorted_relative_paths_and_contents() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let skill = temp.path().join("skills/review");
    std::fs::create_dir_all(skill.join("details")).expect("skill dirs");
    std::fs::write(skill.join("root.md"), "Review carefully.").expect("root");
    std::fs::write(skill.join("details/checks.md"), "Check ownership.").expect("details");
    let hash = directory_sha256(&skill, &["root.md", "details/checks.md"]);
    write_manifest(
        temp.path(),
        &format!(
            r#"
[pack]
schema_version = 1
id = "directory-pack"
version = "1.0.0"

[[resources]]
kind = "skill"
id = "review"
path = "skills/review"
sha256 = "{hash}"
"#
        ),
    );

    let report = CapabilityPackValidator::validate(temp.path());

    assert!(report.valid, "{:#?}", report.issues);
    assert_eq!(report.pack.expect("pack").resources[0].sha256, hash);
}

#[test]
fn strict_manifest_parsing_rejects_unknown_fields() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    write_manifest(
        temp.path(),
        r#"
[pack]
schema_version = 1
id = "strict-pack"
version = "1.0.0"
unexpected = true
"#,
    );

    let report = CapabilityPackValidator::validate(temp.path());

    assert!(report.has_issue(CapabilityPackIssueCode::ManifestParseError));
    assert!(report.pack.is_none());
}
