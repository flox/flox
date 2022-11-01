use anyhow::{anyhow, Result};
use tokio::process::Command;

use crate::environment::{build_flox_env, FLOX_SH, NIX_BIN};

pub(crate) struct CommandRunner {}

impl CommandRunner {
    pub async fn get_templates() -> Result<String> {
        let process = Command::new("nix")
            .arg("eval")
            .arg("--no-write-lock-file")
            .arg("--raw")
            .arg("--apply")
            .arg(
                r#"
        x: with builtins; concatStringsSep "\n" (
            attrValues (mapAttrs (k: v: k + ": " + v.description) (removeAttrs x ["_init"]))
          )
        ' "flox#templates"
        "#,
            )
            .output();

        let output = process.await?;

        Ok(std::str::from_utf8(&output.stdout)?.to_string())
    }

    pub async fn run_in_nix(cmd: &str, args: &Vec<&str>) -> Result<String> {
        let output = Command::new(NIX_BIN)
            .envs(&build_flox_env()?)
            .arg(cmd)
            .args(args)
            .output()
            .await?;

        let nix_response = std::str::from_utf8(&output.stdout)?;
        let nix_err_response = std::str::from_utf8(&output.stderr)?;

        if !nix_err_response.is_empty() {
            println!(
                "Error in nix response, {}, {}",
                nix_err_response,
                nix_err_response.len()
            );
            Err(anyhow!(
                "FXXXX: Error in nix response, {}, {}",
                nix_err_response,
                nix_err_response.len()
            ))
        } else {
            Ok(nix_response.to_string())
        }
    }
}
