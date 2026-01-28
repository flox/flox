use std::fmt::Display;

use crossterm::style::Stylize;

pub fn format_error(v: impl Display) -> String {
    let icon = if stderr_supports_color() {
        "✘".red().to_string()
    } else {
        "✘".to_string()
    };
    format!("{icon} ERROR: {v}")
}

pub fn format_updated(v: impl Display) -> String {
    let icon = if stderr_supports_color() {
        "✔".green().to_string()
    } else {
        "✔".to_string()
    };
    format!("{icon} {v}")
}

pub fn stdout_supports_color() -> bool {
    supports_color::on(supports_color::Stream::Stdout).is_some()
}

pub fn stderr_supports_color() -> bool {
    supports_color::on(supports_color::Stream::Stderr).is_some()
}
