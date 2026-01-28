use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use shell_gen::{GenerateShell, Shell, set_exported_unexpanded, unset};

use crate::env_diff::EnvDiff;

/// Arguments for generating fish startup commands
#[derive(Debug, Clone)]
pub struct FishStartupArgs {
    pub flox_activate_tracelevel: u32,
    pub activate_d: PathBuf,
    pub flox_env: PathBuf,
    pub flox_env_cache: Option<PathBuf>,
    pub flox_env_project: Option<PathBuf>,
    pub flox_env_description: Option<String>,
    pub is_in_place: bool,
    pub clean_up: Option<PathBuf>,

    // Some(_) if it exists, None otherwise
    pub flox_sourcing_rc: bool,
    pub flox_activate_tracer: String,
    pub flox_activations: PathBuf,
}

// N.B. the output of these scripts may be eval'd with backticks which have
// the effect of removing newlines from the output, so we must ensure that
// the output is a valid shell script fragment when represented on a single line.
pub fn generate_fish_startup_commands(
    args: &FishStartupArgs,
    env_diff: &EnvDiff,
    writer: &mut impl Write,
) -> Result<()> {
    let mut stmts = vec![];

    // Enable trace mode if requested
    if args.flox_activate_tracelevel >= 2 {
        stmts.push(set_exported_unexpanded("fish_trace", "1").to_stmt());
    }

    // The fish --init-command option allows us to source our startup
    // file after the normal configuration has been processed, so there
    // is no requirement to go back and source the user's own config
    // as we do in bash.

    // Restore environment variables set in the previous fish initialization.
    env_diff.generate_statements(&mut stmts);

    // Propagate required variables that are documented as exposed.
    stmts.push(set_exported_unexpanded(
        "FLOX_ENV",
        args.flox_env.display().to_string(),
    ));

    // Propagate optional variables that are documented as exposed.
    if let Some(flox_env_cache) = &args.flox_env_cache {
        stmts.push(set_exported_unexpanded(
            "FLOX_ENV_CACHE",
            flox_env_cache.display().to_string(),
        ));
    } else {
        stmts.push(unset("FLOX_ENV_CACHE"));
    }

    if let Some(flox_env_project) = &args.flox_env_project {
        stmts.push(set_exported_unexpanded(
            "FLOX_ENV_PROJECT",
            flox_env_project.display().to_string(),
        ));
    } else {
        stmts.push(unset("FLOX_ENV_PROJECT"));
    }

    if let Some(description) = &args.flox_env_description {
        stmts.push(set_exported_unexpanded("FLOX_ENV_DESCRIPTION", description));
    } else {
        stmts.push(unset("FLOX_ENV_DESCRIPTION"));
    }

    stmts.push(set_exported_unexpanded(
        "_activate_d",
        args.activate_d.display().to_string(),
    ));
    stmts.push(set_exported_unexpanded(
        "_flox_activations",
        args.flox_activations.display().to_string(),
    ));

    stmts.push(set_exported_unexpanded(
        "_flox_activate_tracer",
        &args.flox_activate_tracer,
    ));

    // Set the prompt if we're in an interactive shell.
    let set_prompt_path = args.activate_d.join("set-prompt.fish");
    stmts.push(format!("if isatty 1; source '{}'; end;", set_prompt_path.display()).to_stmt());

    // We already customized the PATH and MANPATH, but the user and system
    // dotfiles may have changed them, so finish by doing this again.

    // fish doesn't have {foo:-} syntax, so we need to provide a temporary variable
    // (foo_with_default) that is either the runtime (not generation-time) value
    // or the string 'empty'.
    stmts.push(
        r#"set -gx FLOX_ENV_DIRS (if set -q FLOX_ENV_DIRS; echo "$FLOX_ENV_DIRS"; else; echo empty; end);"#.to_stmt()
    );

    stmts.push(
        format!(
            r#"{} set-env-dirs --shell fish --flox-env "{}" --env-dirs "$FLOX_ENV_DIRS" | source;"#,
            args.flox_activations.display(),
            args.flox_env.display()
        )
        .to_stmt(),
    );

    stmts.push(
        r#"set -gx MANPATH (if set -q MANPATH; echo "$MANPATH"; else; echo empty; end);"#.to_stmt(),
    );

    stmts.push(format!(
        r#"{} fix-paths --shell fish --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "$MANPATH" | source;"#,
        args.flox_activations.display()
    ).to_stmt());

    stmts.push(
        r#"set -g  _FLOX_SOURCED_PROFILE_SCRIPTS (if set -q _FLOX_SOURCED_PROFILE_SCRIPTS; echo "$_FLOX_SOURCED_PROFILE_SCRIPTS"; else; echo ""; end);"#.to_string()
    .to_stmt());

    stmts.push(format!(
        r#"{} profile-scripts --shell fish --already-sourced-env-dirs  "$_FLOX_SOURCED_PROFILE_SCRIPTS" --env-dirs "$FLOX_ENV_DIRS" | source;"#,
        args.flox_activations.display()
    ).to_stmt());

    // fish does not use hashing in the same way bash does, so there's
    // nothing to be done here by way of that requirement.

    // Disable trace mode if it was enabled
    if args.flox_activate_tracelevel >= 2 {
        stmts.push("set -gx fish_trace 0;".to_stmt());
    }

    if let Some(path) = args.clean_up.as_ref() {
        stmts.push(format!("rm '{}';", path.display()).to_stmt());
    }

    for stmt in stmts {
        stmt.generate_with_newline(Shell::Fish, writer)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use expect_test::expect;

    use super::*;

    // NOTE: For these `expect!` tests, run unit tests with `UPDATE_EXPECT=1`
    //  to have it automatically update the expected value when the implementation
    //  changes.

    #[test]
    fn test_generate_zsh_startup_commands_basic() {
        let additions = {
            let mut map = HashMap::new();
            map.insert("ADDED_VAR".to_string(), "ADDED_VALUE".to_string());
            map
        };
        let deletions = vec!["DELETED_VAR".to_string()];
        let env_diff = EnvDiff::from_parts(additions, deletions);
        let args = FishStartupArgs {
            flox_activate_tracelevel: 3,
            activate_d: PathBuf::from("/activate_d"),
            flox_env: "/flox_env".into(),
            flox_env_cache: Some("/flox_env_cache".into()),
            flox_env_project: Some("/flox_env_project".into()),
            flox_env_description: Some("env_description".to_string()),
            is_in_place: false,
            flox_sourcing_rc: false,
            flox_activate_tracer: "TRACER".into(),
            flox_activations: PathBuf::from("/flox_activations"),
            clean_up: Some("/path/to/rc/file".into()),
        };
        let mut buf = Vec::new();
        generate_fish_startup_commands(&args, &env_diff, &mut buf).unwrap();
        let output = String::from_utf8_lossy(&buf);
        expect![[r#"
            set -gx fish_trace '1';
            set -gx ADDED_VAR 'ADDED_VALUE';
            set -e DELETED_VAR;
            set -gx FLOX_ENV '/flox_env';
            set -gx FLOX_ENV_CACHE '/flox_env_cache';
            set -gx FLOX_ENV_PROJECT '/flox_env_project';
            set -gx FLOX_ENV_DESCRIPTION 'env_description';
            set -gx _activate_d '/activate_d';
            set -gx _flox_activations '/flox_activations';
            set -gx _flox_activate_tracer 'TRACER';
            if isatty 1; source '/activate_d/set-prompt.fish'; end;
            set -gx FLOX_ENV_DIRS (if set -q FLOX_ENV_DIRS; echo "$FLOX_ENV_DIRS"; else; echo empty; end);
            /flox_activations set-env-dirs --shell fish --flox-env "/flox_env" --env-dirs "$FLOX_ENV_DIRS" | source;
            set -gx MANPATH (if set -q MANPATH; echo "$MANPATH"; else; echo empty; end);
            /flox_activations fix-paths --shell fish --env-dirs "$FLOX_ENV_DIRS" --path "$PATH" --manpath "$MANPATH" | source;
            set -g  _FLOX_SOURCED_PROFILE_SCRIPTS (if set -q _FLOX_SOURCED_PROFILE_SCRIPTS; echo "$_FLOX_SOURCED_PROFILE_SCRIPTS"; else; echo ""; end);
            /flox_activations profile-scripts --shell fish --already-sourced-env-dirs  "$_FLOX_SOURCED_PROFILE_SCRIPTS" --env-dirs "$FLOX_ENV_DIRS" | source;
            set -gx fish_trace 0;
            rm '/path/to/rc/file';
        "#]].assert_eq(&output);
    }
}
