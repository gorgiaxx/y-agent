use std::path::Path;

fn main() {
    // Copy builtin skills from the project root into src-tauri/skills/ for
    // Tauri resource bundling. This avoids `../` relative paths in
    // tauri.conf.json which produce `_up_` directory segments on Windows
    // and lose subdirectory structure when using glob patterns.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_path = Path::new(&manifest_dir);

    let project_root = manifest_path
        .parent() // crates/y-gui/
        .and_then(|p| p.parent()) // crates/
        .and_then(|p| p.parent()); // project root

    if let Some(root) = project_root {
        let skills_src = root.join("skills");
        let skills_dst = manifest_path.join("skills");

        // Re-run if the source skills directory changes.
        println!("cargo:rerun-if-changed={}", skills_src.display());

        if skills_src.is_dir() {
            // Remove stale copy and re-create from source.
            let _ = std::fs::remove_dir_all(&skills_dst);
            copy_dir_recursive(&skills_src, &skills_dst);
        }
    }

    tauri_build::build();
}

/// Recursively copy a directory tree, skipping `.DS_Store` files.
fn copy_dir_recursive(src: &Path, dst: &Path) {
    if std::fs::create_dir_all(dst).is_err() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(src) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == ".DS_Store" {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            let _ = std::fs::copy(&src_path, &dst_path);
        }
    }
}
