use anyhow::{Context, bail};
use clap::Args;
use flox_core::activate::context::{InvocationKind, InvocationTypes};
use flox_core::activate::vars::{
    FLOX_INVOCATION_TYPES_PUSH_ENV_VAR,
    FLOX_INVOCATION_TYPES_VAR,
    FLOX_INVOCATION_TYPES_WIRE_VAR,
};

/// Compute the new `_FLOX_INVOCATION_TYPES` value for an activation.
///
/// The variable is deliberately unexported (see
/// [`FLOX_INVOCATION_TYPES_VAR`]), so the activation script passes the
/// current value in and assigns the printed result back. tcsh cannot pass
/// JSON values on a backtick command line and uses the `*-from-env`
/// variants, reading the short-lived exported variables its startup script
/// sets around this call.
#[derive(Debug, Args)]
pub struct PushInvocationTypeArgs {
    #[arg(long, help = "The invocation type of the activation being recorded")]
    pub invocation_type: InvocationKind,
    #[arg(
        long,
        help = "The activated environment's pointer as serialized in _FLOX_ACTIVE_ENVIRONMENTS",
        conflicts_with = "env_from_env"
    )]
    pub env: Option<String>,
    #[arg(
        long,
        help = format!("Read the environment pointer from {FLOX_INVOCATION_TYPES_PUSH_ENV_VAR}")
    )]
    pub env_from_env: bool,
    #[arg(
        long,
        help = "The current _FLOX_INVOCATION_TYPES value",
        conflicts_with = "current_from_env"
    )]
    pub current: Option<String>,
    #[arg(
        long,
        help = format!("Read the current value from {FLOX_INVOCATION_TYPES_WIRE_VAR}")
    )]
    pub current_from_env: bool,
}

impl PushInvocationTypeArgs {
    pub fn handle(&self) -> Result<(), anyhow::Error> {
        let mut output = std::io::stdout();
        self.handle_inner(&mut output)
    }

    fn handle_inner(&self, output: &mut impl std::io::Write) -> Result<(), anyhow::Error> {
        let env_pointer = match (&self.env, self.env_from_env) {
            (Some(env), false) => env.clone(),
            (None, true) => std::env::var(FLOX_INVOCATION_TYPES_PUSH_ENV_VAR).context(format!(
                "{FLOX_INVOCATION_TYPES_PUSH_ENV_VAR} not set in environment"
            ))?,
            _ => bail!("exactly one of --env and --env-from-env must be provided"),
        };
        let current = match (&self.current, self.current_from_env) {
            (Some(current), false) => current.clone(),
            (None, true) => std::env::var(FLOX_INVOCATION_TYPES_WIRE_VAR).context(format!(
                "{FLOX_INVOCATION_TYPES_WIRE_VAR} not set in environment"
            ))?,
            _ => bail!("exactly one of --current and --current-from-env must be provided"),
        };

        let env = serde_json::from_str(&env_pointer)
            .context("could not parse the environment pointer")?;
        let mut invocation_types: InvocationTypes = current
            .parse()
            .context(format!("could not parse {FLOX_INVOCATION_TYPES_VAR}"))?;
        invocation_types.insert_if_absent(env, self.invocation_type);
        write!(output, "{invocation_types}")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: PushInvocationTypeArgs) -> String {
        let mut buf = Vec::new();
        args.handle_inner(&mut buf).expect("handle should succeed");
        String::from_utf8(buf).expect("output should be utf-8")
    }

    fn args(env: &str, current: &str, invocation_type: InvocationKind) -> PushInvocationTypeArgs {
        PushInvocationTypeArgs {
            invocation_type,
            env: Some(env.to_string()),
            env_from_env: false,
            current: Some(current.to_string()),
            current_from_env: false,
        }
    }

    #[test]
    fn inserts_into_an_empty_map() {
        let output = run(args(
            r#"{"name":"default","type":"path"}"#,
            "",
            InvocationKind::InPlace,
        ));
        assert_eq!(
            output,
            r#"[{"env":{"name":"default","type":"path"},"invocation_type":"inplace"}]"#
        );
    }

    #[test]
    fn keeps_the_entry_of_a_repeat_activation() {
        let current =
            r#"[{"env":{"name":"default","type":"path"},"invocation_type":"interactive"}]"#;
        // Key comparison is by value: object key order doesn't matter, so
        // the repeat is recognized (no duplicate entry) and the original
        // `interactive` entry survives the in-place re-activation.
        let output = run(args(
            r#"{"type":"path","name":"default"}"#,
            current,
            InvocationKind::InPlace,
        ));
        assert_eq!(output, current);
    }

    #[test]
    fn rejects_garbage() {
        let mut buf = Vec::new();
        assert!(
            args(r#"{"name":"default"}"#, "bogus", InvocationKind::InPlace)
                .handle_inner(&mut buf)
                .is_err()
        );
        assert!(
            args("not json", "", InvocationKind::InPlace)
                .handle_inner(&mut buf)
                .is_err()
        );
    }
}
