use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use super::Shell;
use crate::{debug_remove_var, debug_set_var};

/// Shell-escape a value for safe use in shell commands.
/// Wraps the value in single quotes and escapes any single quotes within.
fn shell_escape_value(value: &str) -> String {
    // Use single quotes and escape any single quotes in the value
    // by replacing ' with '\''
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Parse a JSON environment file (output of `jq -nS env`) into a HashMap.
///
/// The JSON file should be an object with environment variable names as keys
/// and their values as string values.
fn parse_env_json(path: impl AsRef<Path>) -> Result<HashMap<String, String>> {
    let contents = std::fs::read_to_string(path.as_ref())?;
    let json: Value = serde_json::from_str(&contents)?;

    let obj = json
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected JSON object in environment file"))?;

    let mut env_map = HashMap::new();
    for (key, value) in obj {
        let value_str = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Expected string value for key: {}", key))?;
        env_map.insert(key.clone(), value_str.to_string());
    }

    Ok(env_map)
}

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

impl From<EnvDiff> for ExportEnvDiff {
    fn from(env_diff: EnvDiff) -> Self {
        let mut add_lines: Vec<String> = env_diff
            .additions
            .into_iter()
            .map(|(key, value)| format!("{}={}", key, value))
            .collect();
        add_lines.sort();

        let mut del_lines = env_diff.deletions;
        del_lines.sort();

        Self {
            add: add_lines.join("\n"),
            del: del_lines.join("\n"),
        }
    }
}

impl ExportEnvDiff {
    /// Create an ExportEnvDiff by comparing two JSON environment snapshots.
    ///
    /// # Arguments
    /// * `start_json` - Path to the starting environment JSON (e.g., start.env.json)
    /// * `end_json` - Path to the ending environment JSON (e.g., end.env.json)
    ///
    /// # Returns
    /// An ExportEnvDiff with `add` containing KEY=value lines for variables to set,
    /// and `del` containing variable names to unset.
    pub fn from_files(start_json: impl AsRef<Path>, end_json: impl AsRef<Path>) -> Result<Self> {
        let start_env = parse_env_json(start_json)?;
        let end_env = parse_env_json(end_json)?;

        let mut add_lines = Vec::new();
        let mut del_lines = Vec::new();

        // Find variables to add or update (in end_env but different from start_env)
        for (key, end_value) in &end_env {
            if start_env.get(key) != Some(end_value) {
                add_lines.push(format!("{}={}", key, end_value));
            }
        }

        // Find variables to delete (in start_env but not in end_env)
        for key in start_env.keys() {
            if !end_env.contains_key(key) {
                del_lines.push(key.clone());
            }
        }

        // Sort for consistency
        add_lines.sort();
        del_lines.sort();

        Ok(Self {
            add: add_lines.join("\n"),
            del: del_lines.join("\n"),
        })
    }

    pub fn generate_commands(&self, shell: Shell) -> Vec<String> {
        let mut commands = Vec::new();
        match shell {
            Shell::Bash => {
                for line in self.del.lines() {
                    commands.push(format!("unset {line}"));
                }
                for line in self.add.lines() {
                    if let Some((key, value)) = line.split_once('=') {
                        // Shell-escape the value for safe export
                        let escaped = shell_escape_value(value);
                        commands.push(format!("export {key}={escaped}"));
                    }
                }
            },
            Shell::Fish => {
                for line in self.del.lines() {
                    commands.push(format!("set -e {line}"));
                }
                for line in self.add.lines() {
                    if let Some((key, value)) = line.split_once('=') {
                        // Shell-escape the value for safe export
                        let escaped = shell_escape_value(value);
                        commands.push(format!("set -gx {key} {escaped}"));
                    }
                }
            },
            Shell::Tcsh => {
                for line in self.del.lines() {
                    commands.push(format!("unsetenv {line}"));
                }
                for line in self.add.lines() {
                    if let Some((key, value)) = line.split_once('=') {
                        // Shell-escape the value for safe export
                        let escaped = shell_escape_value(value);
                        commands.push(format!("setenv {key} {escaped}"));
                    }
                }
            },
            Shell::Zsh => {
                for line in self.del.lines() {
                    commands.push(format!("unset {line}"));
                }
                for line in self.add.lines() {
                    if let Some((key, value)) = line.split_once('=') {
                        // Shell-escape the value for safe export
                        let escaped = shell_escape_value(value);
                        commands.push(format!("export {key}={escaped}"));
                    }
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

    // add.env has entries of the form KEY=value
    // Generated by jq -r, values are already unescaped
    fn addition_from_export_line(line: &str) -> Result<(String, String)> {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid export line: {}", line))?;

        // Value is already unescaped by jq -r, use directly
        Ok((key.to_string(), value.to_string()))
    }
}

/// Replay environment variables by comparing two JSON environment snapshots
/// directly in the current Rust process.
///
/// This implements the `replayEnv()` function from the Mermaid diagram.
///
/// # Arguments
/// * `start_json` - Path to the starting environment JSON (e.g., start.env.json)
/// * `end_json` - Path to the ending environment JSON (e.g., end.env.json)
///
/// The function calculates the diff between the two environments and applies:
/// - Variables that exist in start but not in end are unset
/// - Variables that exist in end (either new or changed) are set
///
/// # Safety
/// This function modifies the process environment using unsafe operations.
/// It should be called before any concurrent access to environment variables.
pub fn replay_env(start_json: impl AsRef<Path>, end_json: impl AsRef<Path>) -> Result<()> {
    let start_env = parse_env_json(start_json)?;
    let end_env = parse_env_json(end_json)?;

    // Unset variables that exist in start but not in end
    // Only remove if the variable currently exists in the environment
    for key in start_env.keys() {
        if !end_env.contains_key(key) && std::env::var(key).is_ok() {
            debug_remove_var!(key);
        }
    }

    // Set variables from end (either new or changed from start)
    // Only set if the variable changed during activation AND the current value differs
    for (key, value) in &end_env {
        if start_env.get(key) != Some(value) && std::env::var(key).ok().as_deref() != Some(value.as_str()) {
            debug_set_var!(key, value);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_replay_env_basic() {
        let temp_dir = TempDir::new().unwrap();
        let start_env = temp_dir.path().join("start.env.json");
        let end_env = temp_dir.path().join("end.env.json");

        // Write test files (JSON format from jq -nS env)
        std::fs::write(&start_env, "{}").unwrap();
        std::fs::write(&end_env, r#"{"TEST_VAR":"test_value"}"#).unwrap();

        // Replay environment
        replay_env(&start_env, &end_env).unwrap();

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
        let start_env = temp_dir.path().join("start.env.json");
        let end_env = temp_dir.path().join("end.env.json");

        // Write test files (JSON format with escaped quotes)
        std::fs::write(&start_env, "{}").unwrap();
        std::fs::write(&end_env, r#"{"TEST_QUOTED":"say \"hello\""}"#).unwrap();

        // Replay environment
        replay_env(&start_env, &end_env).unwrap();

        // Check the variable was set with quotes
        assert_eq!(std::env::var("TEST_QUOTED").unwrap(), r#"say "hello""#);

        // Clean up
        unsafe {
            std::env::remove_var("TEST_QUOTED");
        }
    }

    #[test]
    fn test_replay_env_delete() {
        let temp_dir = TempDir::new().unwrap();
        let start_env = temp_dir.path().join("start.env.json");
        let end_env = temp_dir.path().join("end.env.json");

        // Set a variable first
        unsafe {
            std::env::set_var("TEST_DELETE", "value");
        }

        // Write test files - variable in start, not in end
        std::fs::write(&start_env, r#"{"TEST_DELETE":"value"}"#).unwrap();
        std::fs::write(&end_env, "{}").unwrap();

        // Replay environment
        replay_env(&start_env, &end_env).unwrap();

        // Check the variable was deleted
        assert!(std::env::var("TEST_DELETE").is_err());
    }

    #[test]
    fn test_replay_env_add_and_delete() {
        let temp_dir = TempDir::new().unwrap();
        let start_env = temp_dir.path().join("start.env.json");
        let end_env = temp_dir.path().join("end.env.json");

        // Set variables
        unsafe {
            std::env::set_var("TEST_OLD", "old_value");
        }

        // Write test files - TEST_OLD in start, TEST_NEW in end
        std::fs::write(&start_env, r#"{"TEST_OLD":"old_value"}"#).unwrap();
        std::fs::write(&end_env, r#"{"TEST_NEW":"new_value"}"#).unwrap();

        // Replay environment
        replay_env(&start_env, &end_env).unwrap();

        // Check the old variable was deleted and new one was added
        assert!(std::env::var("TEST_OLD").is_err());
        assert_eq!(std::env::var("TEST_NEW").unwrap(), "new_value");

        // Clean up
        unsafe {
            std::env::remove_var("TEST_NEW");
        }
    }
}
