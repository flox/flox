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
