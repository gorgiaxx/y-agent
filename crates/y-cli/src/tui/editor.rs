//! External-editor bridge for multiline composer drafts.

use std::io::Write as _;
use std::process::Command;

pub fn edit(initial: &str) -> Result<String, String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .map_err(|_| "Set VISUAL or EDITOR to use the external editor".to_string())?;
    edit_with_command(initial, &editor)
}

fn edit_with_command(initial: &str, editor: &str) -> Result<String, String> {
    let parts =
        shell_words::split(editor).map_err(|error| format!("Invalid editor command: {error}"))?;
    let (program, args) = parts
        .split_first()
        .ok_or_else(|| "Editor command is empty".to_string())?;
    let mut file = tempfile::Builder::new()
        .prefix("y-agent-draft-")
        .suffix(".md")
        .tempfile()
        .map_err(|error| format!("Could not create editor draft: {error}"))?;
    file.write_all(initial.as_bytes())
        .map_err(|error| format!("Could not write editor draft: {error}"))?;
    file.flush()
        .map_err(|error| format!("Could not flush editor draft: {error}"))?;
    let status = Command::new(program)
        .args(args)
        .arg(file.path())
        .status()
        .map_err(|error| format!("Could not start editor: {error}"))?;
    if !status.success() {
        return Err(format!("Editor exited with status {status}"));
    }
    std::fs::read_to_string(file.path())
        .map_err(|error| format!("Could not read editor draft: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn editor_result_replaces_the_initial_draft() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("editor.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf 'edited\\ntext' > \"$1\"\n").unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&script, permissions).unwrap();

        assert_eq!(
            edit_with_command("original", script.to_str().unwrap()).unwrap(),
            "edited\ntext"
        );
    }

    #[test]
    fn editor_command_supports_quoted_arguments() {
        let parts = shell_words::split("code --wait --name 'Agent Draft'").unwrap();
        assert_eq!(parts, ["code", "--wait", "--name", "Agent Draft"]);
    }
}
