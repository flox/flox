use std::time;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("EOF (End of File): Expected {} but got EOF after reading \"{}\" process terminated with {:?}", .expected, .got, .exit_code.as_ref().unwrap_or(&"unknown".to_owned()))]
    EOF {
        expected: String,
        got: String,
        exit_code: Option<String>,
    },

    #[error("PipeError")]
    BrokenPipe,

    #[error("Timeout Error: Expected {} but got \"{}\" (after waiting {} ms)", .expected, .got, (.timeout.as_secs() * 1000) as u32 + .timeout.subsec_millis())]
    Timeout {
        expected: String,
        got: String,
        timeout: time::Duration,
    },

    #[error("The provided program name is empty.")]
    EmptyProgramName,

    #[error(transparent)]
    Nix(#[from] nix::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("Did not understand Ctrl-{}", .0)]
    SendControlError(char),

    #[error("Failed to send via MPSC channel")]
    MpscSendError,

    #[error(transparent)]
    Regex(#[from] regex::Error),

    #[error("The provided program arguments cannot be parsed")]
    BadProgramArguments,

    #[cfg(feature = "which")]
    #[error(transparent)]
    Which(#[from] which::Error),

    #[error("Dunno")]
    Dunno,
}
