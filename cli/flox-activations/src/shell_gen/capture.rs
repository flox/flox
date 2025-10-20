use std::path::Path;

use anyhow::Result;

use super::Shell;

pub struct EnvDiff {
    add: String,
    del: String,
}

impl EnvDiff {
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
}
