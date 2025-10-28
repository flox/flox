use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use super::Shell;

pub struct ExportEnvDiff {
    add: String,
    del: String,
}

pub struct EnvDiff {
    pub additions: HashMap<String, String>,
    pub deletions: Vec<String>,
}

impl TryFrom<&ExportEnvDiff> for EnvDiff {
    type Error = anyhow::Error;

    fn try_from(export_diff: &ExportEnvDiff) -> Result<Self> {
        let additions = export_diff.additions()?;
        let deletions = export_diff.deletions();

        Ok(EnvDiff {
            additions,
            deletions,
        })
    }
}

impl ExportEnvDiff {
    pub fn from_files(add: impl AsRef<Path>, del: impl AsRef<Path>) -> Result<Self> {
        let add = std::fs::read_to_string(add)?;
        let del = std::fs::read_to_string(del)?;
        Ok(Self { add, del })
    }

    pub fn generate_commands(&self, shell: Shell) -> Vec<String> {
        let mut commands = Vec::new();
        match shell {
            Shell::Bash => {
                for line in self.del.lines() {
                    commands.push(format!("unset {line}"));
                }
                for line in self.add.lines() {
                    commands.push(format!("export {line}"));
                }
            },
            Shell::Fish => {
                for line in self.del.lines() {
                    commands.push(format!("set -e {line}"));
                }
                for line in self.add.lines() {
                    // First replace "=" with space in line.
                    let line = line.replace('=', " ");
                    commands.push(format!("set -gx {line}"));
                }
            },
            Shell::Tcsh => {
                for line in self.del.lines() {
                    commands.push(format!("unsetenv {line}"));
                }
                for line in self.add.lines() {
                    // First replace "=" with space in line.
                    let line = line.replace('=', " ");
                    commands.push(format!("setenv {line}"));
                }
            },
            Shell::Zsh => {
                for line in self.del.lines() {
                    commands.push(format!("unset {line}"));
                }
                for line in self.add.lines() {
                    commands.push(format!("export {line}"));
                }
            },
            _ => unimplemented!(),
        };
        commands
    }

    fn additions(&self) -> Result<HashMap<String, String>> {
        self.add
            .lines()
            .map(Self::addition_from_export_line)
            .collect()
    }

    fn deletions(&self) -> Vec<String> {
        self.del.lines().map(ToString::to_string).collect()
    }

    // add.env has entries of the form
    // FOO="here's some quotes \""
    fn addition_from_export_line(line: &str) -> Result<(String, String)> {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid export line: {}", line))?;

        if !value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            return Err(anyhow::anyhow!("Invalid export line: {}", line));
        }

        let unquoted = &value[1..value.len() - 1];

        // TODO: handle special characters

        Ok((key.to_string(), unquoted.to_string()))
    }
}

/// Replay environment variables from add.env and del.env files directly in the
/// current Rust process.
///
/// This implements the `replayEnv()` function from the Mermaid diagram.
///
/// # File Format
/// - add.env: Lines of the form `VARIABLE="value"` with bash quoting
/// - del.env: Lines with just variable names to unset
///
/// # Safety
/// This function modifies the process environment using unsafe operations.
/// It should be called before any concurrent access to environment variables.
pub fn replay_env(add_env_path: impl AsRef<Path>, del_env_path: impl AsRef<Path>) -> Result<()> {
    // Read the del.env file and unset variables
    if let Ok(del_content) = std::fs::read_to_string(del_env_path.as_ref()) {
        for line in del_content.lines() {
            let line = line.trim();
            if !line.is_empty() {
                unsafe {
                    std::env::remove_var(line);
                }
            }
        }
    }

    // Read the add.env file and set variables
    if let Ok(add_content) = std::fs::read_to_string(add_env_path.as_ref()) {
        for line in add_content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Parse VARIABLE="value" format
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("Invalid add.env line: {}", line))?;

            // Value should be quoted with double quotes
            if !value.starts_with('"') || !value.ends_with('"') || value.len() < 2 {
                return Err(anyhow::anyhow!(
                    "Invalid add.env line (value not properly quoted): {}",
                    line
                ));
            }

            // Remove surrounding quotes
            let quoted_value = &value[1..value.len() - 1];

            // Unescape bash escape sequences
            let unescaped_value = unescape_bash_string(quoted_value)?;

            // Set the environment variable
            unsafe {
                std::env::set_var(key, unescaped_value);
            }
        }
    }

    Ok(())
}

/// Unescape bash escape sequences in a double-quoted string.
///
/// Handles common bash escape sequences:
/// - `\"` -> `"`
/// - `\\` -> `\`
/// - `\n` -> newline
/// - `\t` -> tab
/// - `\r` -> carriage return
/// - `\$` -> `$`
/// - `\`` -> `` ` ``
fn unescape_bash_string(s: &str) -> Result<String> {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('"') => result.push('"'),
                Some('\\') => result.push('\\'),
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('$') => result.push('$'),
                Some('`') => result.push('`'),
                Some(c) => {
                    // For other characters, preserve the backslash
                    result.push('\\');
                    result.push(c);
                },
                None => return Err(anyhow::anyhow!("Trailing backslash in string")),
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_unescape_bash_string_basic() {
        assert_eq!(unescape_bash_string("hello").unwrap(), "hello");
        assert_eq!(unescape_bash_string("").unwrap(), "");
    }

    #[test]
    fn test_unescape_bash_string_quotes() {
        assert_eq!(
            unescape_bash_string(r#"say \"hello\""#).unwrap(),
            r#"say "hello""#
        );
    }

    #[test]
    fn test_unescape_bash_string_backslash() {
        assert_eq!(
            unescape_bash_string(r"path\\to\\file").unwrap(),
            r"path\to\file"
        );
    }

    #[test]
    fn test_unescape_bash_string_special_chars() {
        assert_eq!(
            unescape_bash_string(r"line1\nline2").unwrap(),
            "line1\nline2"
        );
        assert_eq!(unescape_bash_string(r"tab\there").unwrap(), "tab\there");
        assert_eq!(unescape_bash_string(r"\$dollar").unwrap(), "$dollar");
        assert_eq!(unescape_bash_string(r"\`backtick\`").unwrap(), "`backtick`");
    }

    #[test]
    fn test_unescape_bash_string_mixed() {
        assert_eq!(
            unescape_bash_string(r#"say \"hello\"\nworld"#).unwrap(),
            "say \"hello\"\nworld"
        );
    }

    #[test]
    fn test_replay_env_basic() {
        let temp_dir = TempDir::new().unwrap();
        let add_env = temp_dir.path().join("add.env");
        let del_env = temp_dir.path().join("del.env");

        // Write test files
        std::fs::write(&add_env, "TEST_VAR=\"test_value\"\n").unwrap();
        std::fs::write(&del_env, "").unwrap();

        // Replay environment
        replay_env(&add_env, &del_env).unwrap();

        // Check the variable was set
        assert_eq!(std::env::var("TEST_VAR").unwrap(), "test_value");

        // Clean up
        unsafe {
            std::env::remove_var("TEST_VAR");
        }
    }

    #[test]
    fn test_replay_env_with_escapes() {
        let temp_dir = TempDir::new().unwrap();
        let add_env = temp_dir.path().join("add.env");
        let del_env = temp_dir.path().join("del.env");

        // Write test files with escaped quotes
        std::fs::write(&add_env, r#"TEST_QUOTED="say \"hello\"""#).unwrap();
        std::fs::write(&del_env, "").unwrap();

        // Replay environment
        replay_env(&add_env, &del_env).unwrap();

        // Check the variable was set with unescaped quotes
        assert_eq!(std::env::var("TEST_QUOTED").unwrap(), r#"say "hello""#);

        // Clean up
        unsafe {
            std::env::remove_var("TEST_QUOTED");
        }
    }

    #[test]
    fn test_replay_env_delete() {
        let temp_dir = TempDir::new().unwrap();
        let add_env = temp_dir.path().join("add.env");
        let del_env = temp_dir.path().join("del.env");

        // Set a variable first
        unsafe {
            std::env::set_var("TEST_DELETE", "value");
        }

        // Write test files
        std::fs::write(&add_env, "").unwrap();
        std::fs::write(&del_env, "TEST_DELETE\n").unwrap();

        // Replay environment
        replay_env(&add_env, &del_env).unwrap();

        // Check the variable was deleted
        assert!(std::env::var("TEST_DELETE").is_err());
    }

    #[test]
    fn test_replay_env_add_and_delete() {
        let temp_dir = TempDir::new().unwrap();
        let add_env = temp_dir.path().join("add.env");
        let del_env = temp_dir.path().join("del.env");

        // Set variables
        unsafe {
            std::env::set_var("TEST_OLD", "old_value");
        }

        // Write test files
        std::fs::write(&add_env, "TEST_NEW=\"new_value\"\n").unwrap();
        std::fs::write(&del_env, "TEST_OLD\n").unwrap();

        // Replay environment
        replay_env(&add_env, &del_env).unwrap();

        // Check the old variable was deleted and new one was added
        assert!(std::env::var("TEST_OLD").is_err());
        assert_eq!(std::env::var("TEST_NEW").unwrap(), "new_value");

        // Clean up
        unsafe {
            std::env::remove_var("TEST_NEW");
        }
    }
}
