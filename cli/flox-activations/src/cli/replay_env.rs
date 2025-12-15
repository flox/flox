use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use log::debug;

use crate::shell_gen::Shell;
use crate::shell_gen::capture::parse_env_json;

#[derive(Debug, Args)]
pub struct ReplayEnvArgs {
    #[arg(help = "Which shell syntax to return.")]
    #[arg(short, long, value_name = "SHELL")]
    pub shell: String,
    #[arg(
        help = "Path to the activation state directory containing start.env.json and end.env.json."
    )]
    #[arg(long, value_name = "PATH")]
    pub activation_state_dir: PathBuf,
}

impl ReplayEnvArgs {
    pub fn handle_inner(&self, output: &mut impl Write) -> Result<()> {
        let shell: Shell = self.shell.parse()?;

        debug!("Replaying environment changes for shell: {}", shell);

        // Construct paths to the environment snapshots
        let start_json = self.activation_state_dir.join("start.env.json");
        let end_json = self.activation_state_dir.join("end.env.json");

        // Parse the environment snapshots
        let start_env = parse_env_json(&start_json)?;
        let end_env = parse_env_json(&end_json)?;

        let mut commands = Vec::new();

        // Unset variables that exist in start but not in end
        for key in start_env.keys() {
            if !end_env.contains_key(key) {
                debug!("Unsetting variable: {}", key);
                commands.push(shell.unset_var(key));
            }
        }

        // Set variables from end (either new or changed from start)
        for (key, value) in &end_env {
            if start_env.get(key) != Some(value) {
                debug!("Setting variable: {}={}", key, value);
                commands.push(shell.export_var(key, value));
            }
        }

        // Output all commands with semicolons and newlines
        for command in commands {
            writeln!(output, "{};", command)?;
        }

        Ok(())
    }

    pub fn handle(&self) -> Result<()> {
        let mut stdout = std::io::stdout();
        self.handle_inner(&mut stdout)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::Write as IoWrite;

    use tempfile::TempDir;

    use super::*;

    fn create_env_dir(start_vars: HashMap<&str, &str>, end_vars: HashMap<&str, &str>) -> TempDir {
        let dir = TempDir::new().unwrap();

        // Create start.env.json
        let start_json: HashMap<String, String> = start_vars
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let mut start_file = File::create(dir.path().join("start.env.json")).unwrap();
        writeln!(
            start_file,
            "{}",
            serde_json::to_string(&start_json).unwrap()
        )
        .unwrap();

        // Create end.env.json
        let end_json: HashMap<String, String> = end_vars
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let mut end_file = File::create(dir.path().join("end.env.json")).unwrap();
        writeln!(end_file, "{}", serde_json::to_string(&end_json).unwrap()).unwrap();

        dir
    }

    #[test]
    fn test_replay_env_bash_set_new_var() {
        let dir = create_env_dir(HashMap::new(), HashMap::from([("NEW_VAR", "new_value")]));

        let args = ReplayEnvArgs {
            shell: "bash".to_string(),
            activation_state_dir: dir.path().to_path_buf(),
        };

        let mut output = Vec::new();
        args.handle_inner(&mut output).unwrap();

        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("export NEW_VAR='new_value';"));
    }

    #[test]
    fn test_replay_env_bash_unset_var() {
        let dir = create_env_dir(HashMap::from([("OLD_VAR", "old_value")]), HashMap::new());

        let args = ReplayEnvArgs {
            shell: "bash".to_string(),
            activation_state_dir: dir.path().to_path_buf(),
        };

        let mut output = Vec::new();
        args.handle_inner(&mut output).unwrap();

        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("unset OLD_VAR;"));
    }

    #[test]
    fn test_replay_env_fish_syntax() {
        let dir = create_env_dir(
            HashMap::from([("OLD_VAR", "old_value")]),
            HashMap::from([("NEW_VAR", "new_value")]),
        );

        let args = ReplayEnvArgs {
            shell: "fish".to_string(),
            activation_state_dir: dir.path().to_path_buf(),
        };

        let mut output = Vec::new();
        args.handle_inner(&mut output).unwrap();

        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("set -e OLD_VAR;"));
        assert!(result.contains("set -gx NEW_VAR 'new_value';"));
    }

    #[test]
    fn test_replay_env_unchanged_var_not_output() {
        let dir = create_env_dir(
            HashMap::from([("SAME_VAR", "same_value")]),
            HashMap::from([("SAME_VAR", "same_value")]),
        );

        let args = ReplayEnvArgs {
            shell: "bash".to_string(),
            activation_state_dir: dir.path().to_path_buf(),
        };

        let mut output = Vec::new();
        args.handle_inner(&mut output).unwrap();

        let result = String::from_utf8(output).unwrap();
        assert_eq!(result, ""); // No commands should be output
    }
}
