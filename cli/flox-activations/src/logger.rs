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
}
