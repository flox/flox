use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use shell_gen::{GenerateShell, Shell, set_unexported_unexpanded, source_file};

/// Arguments for generating bash startup commands
#[derive(Debug, Clone)]
pub struct ZshStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub flox_env_cache: Option<PathBuf>,
    pub flox_env_project: Option<PathBuf>,
    pub flox_env_description: Option<String>,
    pub is_in_place: bool,
    pub clean_up: Option<PathBuf>,
    pub activation_state_dir: PathBuf,

    // Some(_) if it exists, None otherwise
    pub zshrc_path: Option<PathBuf>,
    pub flox_activate_tracer: String,
    pub flox_sourcing_rc: bool,
    pub flox_activations: PathBuf,
}

pub fn generate_zsh_startup_commands(args: &ZshStartupArgs, writer: &mut impl Write) -> Result<()> {
    ///////////////////////////////////////////////////////////////////////////
    // NOTE: everything between these delimiters is exactly the same as Bash
    //  modulo s/bash/zsh/g

    let mut stmts = vec![];
    stmts.push(set_unexported_unexpanded(
        "_flox_activate_tracelevel",
        format!("{}", &args.flox_activate_tracelevel),
    ));
    stmts.push(set_unexported_unexpanded(
        "_FLOX_ACTIVATION_STATE_DIR",
        args.activation_state_dir.display().to_string(),
    ));
    stmts.push(set_unexported_unexpanded(
        "_activate_d",
        args.activate_d.display().to_string(),
    ));
    // Propagate required variables that are documented as exposed.
    stmts.push(set_unexported_unexpanded(
        "FLOX_ENV",
        args.flox_env.display().to_string(),
    ));
    if let Some(flox_env_cache) = &args.flox_env_cache {
        stmts.push(set_unexported_unexpanded(
            "FLOX_ENV_CACHE",
            flox_env_cache.display().to_string(),
        ));
    }
    if let Some(flox_env_project) = &args.flox_env_project {
        stmts.push(set_unexported_unexpanded(
            "FLOX_ENV_PROJECT",
            flox_env_project.display().to_string(),
        ));
    }
    if let Some(description) = &args.flox_env_description {
        stmts.push(set_unexported_unexpanded(
            "FLOX_ENV_DESCRIPTION",
            description,
        ));
    }
    stmts.push(source_file(args.activate_d.join("zsh")));

    // N.B. the output of these scripts may be eval'd with backticks which have
    // the effect of removing newlines from the output, so we must ensure that
    // the output is a valid shell script fragment when represented on a single line.
    for stmt in stmts {
        stmt.generate_with_newline(Shell::Zsh, writer)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use expect_test::expect;

    use super::*;

    // NOTE: For these `expect!` tests, run unit tests with `UPDATE_EXPECT=1`
    //  to have it automatically update the expected value when the implementation
    //  changes.

    #[test]
    fn test_generate_zsh_startup_commands_basic() {
        let args = ZshStartupArgs {
            flox_activate_tracelevel: 3,
            activate_d: PathBuf::from("/activate_d"),
            activation_state_dir: PathBuf::from("/activation_state_dir"),
            flox_env: "/flox_env".into(),
            flox_env_cache: Some("/flox_env_cache".into()),
            flox_env_project: Some("/flox_env_project".into()),
            flox_env_description: Some("env_description".to_string()),
            is_in_place: false,
            zshrc_path: Some(PathBuf::from("/home/user/.zshrc")),
            flox_sourcing_rc: false,
            flox_activate_tracer: "TRACER".into(),
            flox_activations: PathBuf::from("/flox_activations"),
            clean_up: Some("/path/to/rc/file".into()),
        };
        let mut buf = Vec::new();
        generate_zsh_startup_commands(&args, &mut buf).unwrap();
        let output = String::from_utf8_lossy(&buf);
        expect![[r#"
            typeset -g _flox_activate_tracelevel='3';
            typeset -g _FLOX_ACTIVATION_STATE_DIR='/activation_state_dir';
            typeset -g _activate_d='/activate_d';
            typeset -g FLOX_ENV='/flox_env';
            typeset -g FLOX_ENV_CACHE='/flox_env_cache';
            typeset -g FLOX_ENV_PROJECT='/flox_env_project';
            typeset -g FLOX_ENV_DESCRIPTION='env_description';
            source '/activate_d/zsh';
        "#]]
        .assert_eq(&output);
    }
}
