use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

#[derive(Debug, Args)]
pub struct DumpEnvArgs {
    #[arg(short, long, help = "Output file path (stdout if omitted)")]
    pub output: Option<PathBuf>,
}

impl DumpEnvArgs {
    pub fn handle(&self) -> Result<()> {
        // Collect environment variables without sorting -- the Rust env_diff
        // code uses HashMap so key order doesn't matter.
        let env_map: HashMap<String, String> = std::env::vars().collect();

        match &self.output {
            Some(path) => {
                let file = std::fs::File::create(path)?;
                let mut writer = std::io::BufWriter::new(file);
                serde_json::to_writer(&mut writer, &env_map)?;
                writer.write_all(b"\n")?;
            },
            None => {
                let stdout = std::io::stdout();
                let mut writer = std::io::BufWriter::new(stdout.lock());
                serde_json::to_writer(&mut writer, &env_map)?;
                writer.write_all(b"\n")?;
            },
        }

        Ok(())
    }
}
