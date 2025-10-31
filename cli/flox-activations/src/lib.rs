pub mod cli;
pub mod executive;
pub mod logging;
pub mod process_compose;
pub mod proctitle;
pub mod shell_gen;

pub type Error = anyhow::Error;

/// Macro to set an environment variable in the current process with debug logging.
/// Using a macro ensures the backtrace shows the actual call site, not a wrapper function.
///
/// # Safety
/// This uses unsafe std::env::set_var internally. The caller must ensure proper synchronization.
///
/// # Examples
/// ```ignore
/// debug_set_var!("MY_VAR", "my_value");
/// debug_set_var!("PATH", computed_path);
/// ```
#[macro_export]
macro_rules! debug_set_var {
    ($key:expr, $value:expr) => {{
        let key = $key;
        let value = $value;
        log::debug!("Setting env var: {}={}", key, value);
        unsafe {
            std::env::set_var(key, value);
        }
    }};
}

/// Macro to set an environment variable on a Command object with debug logging.
/// Using a macro ensures the backtrace shows the actual call site, not a wrapper function.
///
/// Returns the &mut Command to allow chaining.
///
/// # Examples
/// ```ignore
/// debug_command_env!(&mut cmd, "MY_VAR", "my_value");
/// debug_command_env!(&mut cmd, "PATH", computed_path);
/// ```
#[macro_export]
macro_rules! debug_command_env {
    ($cmd:expr, $key:expr, $value:expr) => {{
        let key = $key;
        let value = $value;
        log::debug!("Setting command env var: {}={}", key, value);
        $cmd.env(key, value)
    }};
}
