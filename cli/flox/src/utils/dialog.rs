use std::fmt::Display;
use std::time::{Duration, Instant};

use crossterm::tty::IsTty;
use indicatif::{ProgressBar, ProgressStyle};
use inquire::ui::{Attributes, RenderConfig, StyleSheet, Styled};

use super::{colors, TERMINAL_STDERR};

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct Confirm {
    pub default: Option<bool>,
}
#[derive(Clone)]
pub struct Select<T> {
    pub options: Vec<T>,
}

pub struct Spinner<F>(F);
impl<F: FnOnce() -> T + Send, T: Send> Spinner<F> {
    pub fn new(f: F) -> Self {
        Self(f)
    }
}

#[derive(Debug, Clone)]
pub struct Checkpoint;

#[derive(Debug, Clone)]
pub struct Dialog<'a, Type> {
    pub message: &'a str,
    pub help_message: Option<&'a str>,
    pub typed: Type,
}

impl<'a> Dialog<'a, Confirm> {
    #[allow(unused)]
    pub async fn prompt(self) -> inquire::error::InquireResult<bool> {
        let message = self.message.to_owned();
        let help_message: Option<String> = self.help_message.map(ToOwned::to_owned);
        let default = self.typed.default;

        tokio::task::spawn_blocking(move || {
            let _stderr_lock = TERMINAL_STDERR.lock();

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

impl Dialog<'_, Checkpoint> {
    /// Print the message and wait for the user to press enter
    pub fn checkpoint(self) -> inquire::error::InquireResult<()> {
        let message = self.message;
        let help_message = self.help_message;

        let _stderr_lock = TERMINAL_STDERR.lock();

        let dialog = inquire::CustomType {
            message,
            default: None,
            placeholder: None,
            help_message,
            formatter: &|()| "".to_string(),
            default_value_formatter: &|()| "".to_string(),
            parser: &|_| Ok(()),
            validators: vec![],
            error_message: "".to_string(),
            render_config: flox_theme(),
        };

        dialog.prompt()
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
            let _stderr_lock = TERMINAL_STDERR.lock();

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

    pub fn raw_prompt(self) -> inquire::error::InquireResult<(usize, T)> {
        let message = self.message.to_owned();
        let help_message = self.help_message.map(ToOwned::to_owned);
        let mut options = self.typed.options;

        let choices = options
            .iter()
            .map(ToString::to_string)
            .enumerate()
            .map(|(id, value)| Choice(id, value))
            .collect();

        let (raw_id, Choice(id, _)) = {
            let _stderr_lock = TERMINAL_STDERR.lock();

            let mut dialog =
                inquire::Select::new(&message, choices).with_render_config(flox_theme());

            if let Some(ref help_message) = help_message {
                dialog = dialog.with_help_message(help_message);
            }

            match dialog.raw_prompt() {
                Ok(x) => Ok((x.index, x.value)),
                Err(err) => Err(err),
            }
        }?;

        Ok((raw_id, options.remove(id)))
    }
}

impl<'a, F: FnOnce() -> T + Send, T: Send> Dialog<'a, Spinner<F>> {
    pub fn spin_with_delay(self, start_spinning_after: Duration) -> T {
        let handle = tokio::runtime::Handle::current();
        std::thread::scope(|s| {
            let y = s.spawn(move || {
                // self.typed.0 may be a function that requires tokio
                let _guard = handle.enter();
                (self.typed.0)()
            });
            let mut dialog: Option<ProgressBar> = None;
            let started = Instant::now();
            loop {
                if y.is_finished() {
                    break;
                }

                if Instant::now() - started < start_spinning_after {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }

                let spinner = indicatif::ProgressBar::new_spinner();
                spinner.set_style(
                    ProgressStyle::with_template("{spinner} {wide_msg} {prefix:>}").unwrap(),
                );
                spinner.set_message(self.message.to_string());
                if let Some(help_message) = self.help_message {
                    spinner.set_prefix(help_message.to_string())
                }
                spinner.enable_steady_tick(Duration::from_millis(100));
                dialog = Some(spinner);

                break;
            }
            let res = y.join().unwrap();

            if let Some(dialog) = dialog {
                dialog.finish_and_clear();
            }

            res
        })
    }

    #[allow(unused)]
    pub fn spin(self) -> T {
        self.spin_with_delay(Duration::from_millis(0))
    }
}

impl Dialog<'_, ()> {
    /// True if stderr and stdin are ttys
    pub fn can_prompt() -> bool {
        if std::env::var("_FLOX_NO_PROMPT").is_ok_and(|v| v == "1") {
            return false;
        }
        std::io::stderr().is_tty() && std::io::stdin().is_tty() && std::io::stdout().is_tty()
    }
}

pub fn flox_theme() -> RenderConfig {
    let mut render_config = RenderConfig::default_colored();

    if let (Some(dark_peach), Some(light_blue)) = (
        colors::INDIGO_300.to_inquire(),
        colors::INDIGO_400.to_inquire(),
    ) {
        render_config.answered_prompt_prefix = Styled::new(">").with_fg(dark_peach);
        render_config.highlighted_option_prefix = Styled::new(">").with_fg(dark_peach);
        render_config.prompt_prefix = Styled::new("!").with_fg(dark_peach);
        render_config.prompt = StyleSheet::new().with_attr(Attributes::BOLD);
        render_config.help_message = Styled::new("").with_fg(light_blue).style;
        render_config.answer = Styled::new("").with_fg(dark_peach).style;
    } else {
        render_config.answered_prompt_prefix = Styled::new(">");
        render_config.highlighted_option_prefix = Styled::new(">");
        render_config.prompt_prefix = Styled::new("!");
        render_config.prompt = StyleSheet::new();
        render_config.help_message = Styled::new("").style;
        render_config.answer = Styled::new("").style;
    }

    render_config
}
