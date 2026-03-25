use std::fmt::Display;
use std::io::IsTerminal;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use futures::StreamExt;
use inquire::ui::{Attributes, RenderConfig, StyleSheet, Styled};

use super::{TERMINAL_STDERR, colors, message};

/// Outcome of waiting for the user to press Enter.
#[derive(Debug, PartialEq, Eq)]
pub enum WaitResult {
    /// The user pressed Enter.
    Enter,
    /// The user pressed Ctrl-C.
    Interrupted,
}

/// RAII guard that disables terminal raw mode on drop.
///
/// Ensures `disable_raw_mode()` is called even if the caller panics,
/// preventing the terminal from being left in a corrupted state.
struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(RawModeGuard)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        // Best-effort: ignore errors on cleanup
        let _ = terminal::disable_raw_mode();
    }
}

/// Wait for the user to press Enter or Ctrl-C.
///
/// Returns [`WaitResult::Enter`] when Enter is pressed,
/// or [`WaitResult::Interrupted`] when Ctrl-C is pressed or the
/// event stream ends unexpectedly.
async fn wait_for_enter() -> WaitResult {
    // Enable raw mode so we receive individual keystrokes.
    // The guard ensures raw mode is disabled on any exit path.
    let _guard = match RawModeGuard::enable() {
        Ok(g) => g,
        Err(_) => return WaitResult::Interrupted,
    };

    let mut reader = EventStream::new();

    while let Some(event) = reader.next().await {
        match event {
            Ok(Event::Key(KeyEvent {
                code: KeyCode::Enter,
                ..
            })) => return WaitResult::Enter,
            Ok(Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers,
                ..
            })) if modifiers.contains(KeyModifiers::CONTROL) => {
                return WaitResult::Interrupted;
            },
            _ => {},
        }
    }

    // Stream ended without a recognized key — treat as interruption.
    WaitResult::Interrupted
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct Confirm {
    pub default: Option<bool>,
}
#[derive(Clone)]
pub struct Select<T> {
    pub options: Vec<T>,
}

/// Marker type for a dialog that waits for the user to press Enter.
#[derive(Debug, Clone)]
pub struct Checkpoint;

#[derive(Debug, Clone)]
pub struct Dialog<'a, Type> {
    pub message: &'a str,
    pub help_message: Option<&'a str>,
    pub typed: Type,
}

impl Dialog<'_, Checkpoint> {
    /// Print the dialog message and wait for the user to press Enter.
    ///
    /// Returns [`WaitResult::Enter`] when Enter is pressed,
    /// or [`WaitResult::Interrupted`] when Ctrl-C is pressed.
    pub async fn checkpoint_async(self) -> WaitResult {
        message::plain(self.message);
        wait_for_enter().await
    }
}

impl Dialog<'_, Confirm> {
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

struct Choice(usize, String);
impl Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.1.fmt(f)
    }
}

impl<T: Display> Dialog<'_, Select<T>> {
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

impl Dialog<'_, ()> {
    /// True if stderr and stdin are ttys
    pub fn can_prompt() -> bool {
        std::io::stderr().is_terminal()
            && std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal()
    }
}

pub fn flox_theme() -> RenderConfig<'static> {
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
