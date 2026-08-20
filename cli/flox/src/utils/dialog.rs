use std::fmt::Display;
use std::io::{IsTerminal, Write};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::{QueueableCommand, cursor, terminal};
use futures::StreamExt;
use inquire::ui::{Attributes, RenderConfig, StyleSheet, Styled};

use super::{TERMINAL_STDERR, colors};

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
pub async fn wait_for_enter() -> WaitResult {
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

#[derive(Debug, Clone)]
pub struct Confirm {
    pub default: Option<bool>,
}
#[derive(Clone)]
pub struct Select<T> {
    pub options: Vec<T>,
}

#[derive(Debug, Clone)]
pub struct Dialog<'a, Type> {
    pub message: &'a str,
    pub help_message: Option<&'a str>,
    pub typed: Type,
}

/// Terminal rows `text` occupies when printed at `width` columns, accounting
/// for line wrapping. Column counting is by `char`, which is exact for the
/// ASCII text this is used on.
fn visual_rows(text: &str, width: u16) -> u16 {
    let width = width.max(1);
    text.split('\n')
        .map(|line| (line.chars().count().max(1) as u16).div_ceil(width))
        .sum()
}

/// A block of stderr lines that is erased once the interaction it supported
/// has finished, so multi-line transient UI (a login code, a consent
/// explainer) collapses into a single summary line in the scrollback.
///
/// Row accounting assumes nothing else prints below the block while it is on
/// screen. A caller that does print below it (e.g. an unexpected warning)
/// must drop the block instead of calling [`TransientBlock::erase`].
#[derive(Debug)]
pub struct TransientBlock {
    texts: Vec<String>,
}

impl TransientBlock {
    /// Print `text` to stderr and start tracking it.
    ///
    /// The block is written directly to stderr rather than through the
    /// message tracing layer: like inquire's own rendering, it is
    /// interactive terminal UI tied to a prompt, so verbosity filters must
    /// not drop it while the prompt still shows, and [`TransientBlock::erase`]
    /// must know exactly what reached the terminal.
    pub fn print(text: &str) -> Self {
        {
            let _stderr_lock = TERMINAL_STDERR.lock();
            let _ = writeln!(std::io::stderr(), "{text}");
        }
        TransientBlock {
            texts: vec![text.to_string()],
        }
    }

    /// Account for a line rendered below the block by someone else
    /// (e.g. a prompt's answered line).
    pub fn track(&mut self, text: &str) {
        self.texts.push(text.to_string());
    }

    /// Erase the tracked rows from the terminal.
    ///
    /// Rows are computed here, with the current terminal width, so that a
    /// resize during the interaction stays accurate on terminals that
    /// reflow soft-wrapped lines (the common behavior).
    ///
    /// No-op when stderr is not a tty: the block then simply remains in the
    /// output, which is the right behavior for logs and CI.
    pub fn erase(self) {
        if !std::io::stderr().is_terminal() {
            return;
        }
        let width = terminal::size().map(|(w, _)| w).unwrap_or(80);
        let rows: u16 = self.texts.iter().map(|text| visual_rows(text, width)).sum();
        if rows == 0 {
            return;
        }
        // Hold the same stderr lock the tracing layer and inquire use.
        // Erasure is cosmetic: failures are ignored, leaving the block on
        // screen.
        let _stderr_lock = TERMINAL_STDERR.lock();
        let mut stderr = std::io::stderr();
        let _ = stderr
            .queue(cursor::MoveToPreviousLine(rows))
            .and_then(|s| s.queue(terminal::Clear(terminal::ClearType::FromCursorDown)))
            .and_then(|s| s.flush());
    }
}

impl Dialog<'_, Confirm> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_rows_counts_wrapped_and_empty_lines() {
        // One row per line that fits, an extra row per wrap, and empty lines
        // still occupy a row.
        assert_eq!(visual_rows("short", 80), 1);
        assert_eq!(visual_rows("", 80), 1);
        assert_eq!(visual_rows("a\nb\nc", 80), 3);
        assert_eq!(visual_rows(&"x".repeat(80), 80), 1);
        assert_eq!(visual_rows(&"x".repeat(81), 80), 2);
        assert_eq!(visual_rows(&format!("{}\nshort", "x".repeat(200)), 80), 4);
    }
}
