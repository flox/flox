use anyhow::{Context, anyhow};
use flox_core::activate::vars::FLOX_ACTIVATIONS_VERBOSITY_VAR;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Verbosity {
    inner: u32,
}

impl From<u32> for Verbosity {
    fn from(value: u32) -> Self {
        Self { inner: value }
    }
}

impl Verbosity {
    pub fn env_filter(&self) -> &'static str {
        match self.inner {
            0 => "flox_activations=error",
            1 => "flox_activations=debug",
            2 => "flox_activations=trace",
            _ => "flox_activations=trace",
        }
    }

    pub fn filter_from_env_and_arg(arg: Option<u32>) -> Option<String> {
        let rust_log = std::env::var("RUST_LOG").context("RUST_LOG not present");
        let our_variable = std::env::var(FLOX_ACTIVATIONS_VERBOSITY_VAR)
            .context("verbosity variable not present")
            .and_then(|value| {
                value
                    .parse::<u32>()
                    .context("failed to parse verbosity as int")
                    .map(Verbosity::from)
                    .map(|v| v.env_filter().to_string())
            });
        let explicit_arg = arg.map(Verbosity::from).map(|v| v.env_filter().to_string());
        let filter = rust_log
            .or(our_variable)
            .or(explicit_arg.ok_or(anyhow!("no arg provided")));
        filter.ok()
    }
}
