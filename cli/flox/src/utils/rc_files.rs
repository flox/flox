use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use shell_gen::ShellWithPath;

/// Locate the RC file for a given shell and filename.
pub fn locate_rc_file(shell: &ShellWithPath, name: impl AsRef<str>) -> Result<PathBuf> {
    use ShellWithPath::*;
    let home = dirs::home_dir().context("failed to locate home directory")?;
    let rc_file = match shell {
        Bash(_) => home.join(name.as_ref()),
        Zsh(_) => home.join(name.as_ref()),
        Tcsh(_) => home.join(name.as_ref()),
        // Note, this `.config` is _not_ what you get from `dirs::config_dir`,
        // which points at `Application Support`
        Fish(_) => home.join(".config/fish").join(name.as_ref()),
    };
    Ok(rc_file)
}

/// Ensure a RC file exists, creating parent directories and the file if needed.
pub fn ensure_rc_file_exists(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if !path.exists() {
        std::fs::create_dir_all(path.parent().context("RC file had no parent")?)
            .context("failed to create parent directory for RC file")?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .context("failed to create empty RC file")?;
    }
    Ok(())
}

/// Append a command to an RC file, creating a backup first.
pub fn add_activation_to_rc_file(path: impl AsRef<Path>, cmd: impl AsRef<str>) -> Result<()> {
    let backup = path.as_ref().with_extension(".pre_flox");
    if backup.exists() {
        std::fs::remove_file(&backup).context("failed to remove old backup of RC file")?;
    }
    std::fs::copy(&path, backup).context("failed to make backup of RC file")?;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .context("failed to open RC file")?;
    file.write(format!("{}\n", cmd.as_ref()).as_bytes())
        .context("failed to write to RC file")?;
    Ok(())
}

/// Return the RC file names for a given shell.
pub fn rc_file_names_for_shell(shell: &ShellWithPath) -> Vec<&'static str> {
    match shell {
        ShellWithPath::Bash(_) => vec![".bashrc", ".profile"],
        ShellWithPath::Zsh(_) => vec![".zshrc", ".zprofile"],
        ShellWithPath::Tcsh(_) => vec![".tcshrc"],
        ShellWithPath::Fish(_) => vec!["config.fish"],
    }
}

/// Return the shell-appropriate `flox activate` command for dotfile use.
pub fn activate_command_for_shell(shell: &ShellWithPath) -> String {
    match shell {
        ShellWithPath::Bash(_) | ShellWithPath::Zsh(_) => r#"eval "$(flox activate)""#.to_string(),
        ShellWithPath::Tcsh(_) => r#"eval "`flox activate`""#.to_string(),
        ShellWithPath::Fish(_) => "flox activate | source".to_string(),
    }
}
