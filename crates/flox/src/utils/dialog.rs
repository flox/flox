use std::fmt::Display;

use crossterm::tty::IsTty;
use inquire::ui::{Attributes, RenderConfig, StyleSheet, Styled};

use super::{colors, TERMINAL_STDERR};

pub struct Text<'a> {
    pub default: Option<&'a str>,
}

#[allow(unused)]
pub struct Confirm {
    pub default: Option<bool>,
}
pub struct Select<T> {
    pub options: Vec<T>,
}

pub struct Dialog<'a, Type> {
    pub message: &'a str,
    pub help_message: Option<&'a str>,
    pub typed: Type,
}

impl<'a> Dialog<'a, Text<'a>> {
    pub async fn prompt(self) -> inquire::error::InquireResult<String> {
        let message = self.message.to_owned();
        let help_message = self.help_message.map(ToOwned::to_owned);
        let default = self.typed.default.map(ToOwned::to_owned);

        tokio::task::spawn_blocking(move || {
            let _stderr_lock = TERMINAL_STDERR.blocking_lock();

            let mut dialog = inquire::Text::new(&message).with_render_config(flox_theme());

            if let Some(ref help_message) = help_message {
                dialog = dialog.with_help_message(help_message);
            }

            if let Some(ref default) = default {
                dialog = dialog.with_initial_value(default);
            }

            dialog.prompt()
        })
        .await
        .expect("Failed to join blocking dialog")
    }
}

impl<'a> Dialog<'a, Confirm> {
    #[allow(unused)]
    pub async fn prompt(self) -> inquire::error::InquireResult<bool> {
        let message = self.message.to_owned();
        let help_message = self.help_message.map(ToOwned::to_owned);
        let default = self.typed.default;

        tokio::task::spawn_blocking(move || {
            let _stderr_lock = TERMINAL_STDERR.blocking_lock();

            let mut dialog = inquire::Confirm::new(&message).with_render_config(flox_theme());

            if let Some(default) = default {
                dialog = dialog.with_default(default);
            }

            if let Some(ref help_message) = help_message {
                dialog = dialog.with_help_message(help_message);
            }

            dialog.prompt()
        })
        .await
        .expect("Failed to join blocking dialog")
    }
}

struct Choice(usize, String);
impl Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.1.fmt(f)
    }
}

impl<'a, T: Display> Dialog<'a, Select<T>> {
    #[allow(dead_code)]
    pub async fn prompt(self) -> inquire::error::InquireResult<T> {
        let message = self.message.to_owned();
        let help_message = self.help_message.map(ToOwned::to_owned);
        let mut options = self.typed.options;

        let choices = options
            .iter()
            .map(ToString::to_string)
            .enumerate()
            .map(|(id, value)| Choice(id, value))
            .collect();

        let Choice(id, _) = tokio::task::spawn_blocking(move || {
            let _stderr_lock = TERMINAL_STDERR.blocking_lock();

            let mut dialog =
                inquire::Select::new(&message, choices).with_render_config(flox_theme());

            if let Some(ref help_message) = help_message {
                dialog = dialog.with_help_message(help_message);
            }

            dialog.prompt()
        })
        .await
        .expect("Failed to join blocking dialog")?;

        Ok(options.remove(id))
    }

    pub async fn raw_prompt(self) -> inquire::error::InquireResult<(usize, T)> {
        let message = self.message.to_owned();
        let help_message = self.help_message.map(ToOwned::to_owned);
        let mut options = self.typed.options;

        let choices = options
            .iter()
            .map(ToString::to_string)
            .enumerate()
            .map(|(id, value)| Choice(id, value))
            .collect();

        let (raw_id, Choice(id, _)) = tokio::task::spawn_blocking(move || {
            let _stderr_lock = TERMINAL_STDERR.blocking_lock();

            let mut dialog =
                inquire::Select::new(&message, choices).with_render_config(flox_theme());

            if let Some(ref help_message) = help_message {
                dialog = dialog.with_help_message(help_message);
            }

            match dialog.raw_prompt() {
                Ok(x) => Ok((x.index, x.value)),
                Err(err) => Err(err),
            }
        })
        .await
        .expect("Failed to join blocking dialog")?;

        Ok((raw_id, options.remove(id)))
    }
}

impl Dialog<'_, ()> {
    /// True if stderr and stdin are ttys
    pub fn can_prompt() -> bool {
        std::io::stderr().is_tty() && std::io::stdin().is_tty()
    }
}

pub fn flox_theme() -> RenderConfig {
    let mut render_config = RenderConfig::default_colored();

    if let (Some(light_peach), Some(light_blue)) = (
        colors::LIGHT_PEACH.to_inquire(),
        colors::LIGHT_BLUE.to_inquire(),
    ) {
        render_config.answered_prompt_prefix = Styled::new(">").with_fg(light_peach);
        render_config.highlighted_option_prefix = Styled::new(">").with_fg(light_peach);
        render_config.prompt_prefix = Styled::new("?").with_fg(light_peach);
        render_config.prompt = StyleSheet::new().with_attr(Attributes::BOLD);
        render_config.help_message = Styled::new("").with_fg(light_blue).style;
        render_config.answer = Styled::new("").with_fg(light_peach).style;
    } else {
        render_config.answered_prompt_prefix = Styled::new(">");
        render_config.highlighted_option_prefix = Styled::new(">");
        render_config.prompt_prefix = Styled::new("?");
        render_config.prompt = StyleSheet::new();
        render_config.help_message = Styled::new("").style;
        render_config.answer = Styled::new("").style;
    }

    render_config
}
