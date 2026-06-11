//! Interactive resolver for the build-time virtual sandbox prompt broker
//! (Phase 2, increment 3).
//!
//! The broker ([`flox_rust_sdk::providers::sandbox_prompt`]) calls a
//! [`PromptResolver`] for each genuinely-new out-of-closure access. This module
//! provides the interactive one: a full-screen "Flox sandbox" page drawn on
//! `/dev/tty` with an arrow-navigable menu offering
//!
//!   - Allow once (this exact file)
//!   - Allow `<dir>/**` at one of several nesting levels (the rollup that
//!     silences a flood and can be written back to `sandbox-allow`)
//!   - Deny (this access)
//!   - Deny all (stop prompting for the rest of the build)
//!
//! Rendering on `/dev/tty` (rather than stdout) means the prompt works even when
//! the build's stdout/stderr are redirected, and crossterm reads key events from
//! the controlling terminal. When there is no usable tty the resolver denies
//! (and a later audit phase can recommend a `sandbox-allow` block instead).
//!
//! Box-drawing glyphs are written with `\u{...}` escapes so the source stays
//! ASCII and the bytes emitted are unambiguous UTF-8.

// Nothing constructs InteractivePromptResolver yet — it is wired into the
// `flox build` path in increment 4, at which point this allow is removed.
#![allow(dead_code)]

use std::fs::File;
use std::io::Write;
use std::path::Path;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, read};
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{
    Clear,
    ClearType,
    EnterAlternateScreen,
    LeaveAlternateScreen,
    disable_raw_mode,
    enable_raw_mode,
    size,
};
use crossterm::{cursor, execute, queue};
use flox_rust_sdk::providers::sandbox_prompt::{PromptDecision, PromptResolver};

// Box-drawing and UI glyphs (escaped to keep the source ASCII).
const TOP_LEFT: char = '\u{250C}'; // top-left corner
const TOP_RIGHT: char = '\u{2510}'; // top-right corner
const BOTTOM_LEFT: char = '\u{2514}'; // bottom-left corner
const BOTTOM_RIGHT: char = '\u{2518}'; // bottom-right corner
const VERTICAL: char = '\u{2502}'; // vertical edge
const HORIZONTAL: char = '\u{2500}'; // horizontal edge
const CURSOR: char = '\u{25B8}'; // selected-row marker
const ELLIPSIS: char = '\u{2026}'; // truncation marker
const HINT: &str = "  \u{2191}/\u{2193} move \u{00B7} \u{21B5} select \u{00B7} Esc deny";

/// What a single menu entry maps to once chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    AllowOnce,
    AllowGlob(String),
    Deny,
    DenyAll,
}

/// A rendered menu entry: the label shown and the choice it selects.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuItem {
    label: String,
    choice: Choice,
}

/// Express `dir` relative to `$HOME` as `~/...` when it lives under it, matching
/// how the patterns are written in `sandbox-allow` (and expanded by libsandbox).
fn home_relative(dir: &str, home: Option<&str>) -> String {
    if let Some(home) = home {
        if dir == home {
            return "~".to_string();
        }
        if let Some(rest) = dir.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    dir.to_string()
}

/// Build the `<dir>/**` rollup patterns offered for `path`, from the immediate
/// parent directory upward. Patterns under `$HOME` are written `~/...`; the walk
/// stops just below `$HOME` (never offering the whole home directory) and at the
/// filesystem root, and is capped so the menu stays manageable for deep paths.
fn ancestor_globs(path: &Path, home: Option<&str>) -> Vec<String> {
    const MAX_LEVELS: usize = 8;
    let mut globs = Vec::new();
    let mut current = path.parent();
    while let Some(dir) = current {
        let dir_str = dir.to_string_lossy();
        if dir_str.is_empty() || dir_str == "/" {
            break;
        }
        let relative = home_relative(&dir_str, home);
        // `~` means we've reached $HOME itself: `~/**` (all of home) is too broad
        // to offer, so stop here.
        if relative == "~" {
            break;
        }
        globs.push(format!("{relative}/**"));
        if globs.len() >= MAX_LEVELS {
            break;
        }
        current = dir.parent();
    }
    globs
}

/// Assemble the full menu for an access to `path`.
fn build_menu(path: &Path, home: Option<&str>) -> Vec<MenuItem> {
    let mut items = vec![MenuItem {
        label: "Allow once (this file)".to_string(),
        choice: Choice::AllowOnce,
    }];
    for glob in ancestor_globs(path, home) {
        items.push(MenuItem {
            label: format!("Allow {glob}"),
            choice: Choice::AllowGlob(glob),
        });
    }
    items.push(MenuItem {
        label: "Deny".to_string(),
        choice: Choice::Deny,
    });
    items.push(MenuItem {
        label: "Deny all (stop prompting)".to_string(),
        choice: Choice::DenyAll,
    });
    items
}

/// Map a chosen [`Choice`] to a [`PromptDecision`], updating `deny_all` when the
/// user asks to stop prompting. `None` (cancelled / no tty) denies.
fn decision_for(choice: Option<&Choice>, deny_all: &mut bool) -> PromptDecision {
    match choice {
        Some(Choice::AllowOnce) => PromptDecision::Allow,
        Some(Choice::AllowGlob(glob)) => PromptDecision::AllowGlob(glob.clone()),
        Some(Choice::Deny) | None => PromptDecision::Deny,
        Some(Choice::DenyAll) => {
            *deny_all = true;
            PromptDecision::Deny
        },
    }
}

/// The interactive [`PromptResolver`]. Stateful: once the user picks "Deny all",
/// every later access is denied without prompting.
pub struct InteractivePromptResolver {
    home: Option<String>,
    deny_all: bool,
}

impl Default for InteractivePromptResolver {
    fn default() -> Self {
        Self {
            home: std::env::var("HOME").ok(),
            deny_all: false,
        }
    }
}

impl InteractivePromptResolver {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PromptResolver for InteractivePromptResolver {
    fn resolve(&mut self, path: &Path) -> PromptDecision {
        if self.deny_all {
            return PromptDecision::Deny;
        }
        let menu = build_menu(path, self.home.as_deref());
        // A missing/erroring tty (non-interactive) yields None: fail closed by
        // denying. A later audit phase can recommend a sandbox-allow block.
        let selected = prompt_on_tty(path, &menu).unwrap_or_default();
        decision_for(selected.map(|i| &menu[i].choice), &mut self.deny_all)
    }
}

/// Draw the menu full-screen on `/dev/tty` and return the index the user
/// selected, or `None` if they cancelled (Esc / Ctrl-C / `q`). Errors only when
/// `/dev/tty` or raw mode is unavailable (non-interactive), which the caller
/// treats as "deny".
fn prompt_on_tty(path: &Path, menu: &[MenuItem]) -> std::io::Result<Option<usize>> {
    // Render to /dev/tty so the prompt is visible even with stdout/stderr
    // redirected. crossterm reads key events from the controlling terminal.
    let mut tty = File::options().read(true).write(true).open("/dev/tty")?;

    enable_raw_mode()?;
    execute!(tty, EnterAlternateScreen, cursor::Hide)?;

    // Restore the terminal no matter how we leave (selection, cancel, error).
    let result = run_menu_loop(&mut tty, path, menu);

    let _ = execute!(tty, cursor::Show, LeaveAlternateScreen);
    let _ = disable_raw_mode();
    result
}

/// The render + key-handling loop, factored out so `prompt_on_tty` can always
/// restore the terminal afterward.
fn run_menu_loop(tty: &mut File, path: &Path, menu: &[MenuItem]) -> std::io::Result<Option<usize>> {
    let mut selected = 0usize;
    loop {
        draw(tty, path, menu, selected)?;
        let Event::Key(key) = read()? else {
            continue;
        };
        // crossterm reports press and release on some platforms; act on press.
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                selected = selected.checked_sub(1).unwrap_or(menu.len() - 1);
            },
            KeyCode::Down | KeyCode::Char('j') => {
                selected = (selected + 1) % menu.len();
            },
            KeyCode::Enter => return Ok(Some(selected)),
            KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(None);
            },
            _ => {},
        }
    }
}

/// Draw the bordered "Flox sandbox" box with the access path and the menu, the
/// `selected` row highlighted.
fn draw(tty: &mut File, path: &Path, menu: &[MenuItem], selected: usize) -> std::io::Result<()> {
    let (term_cols, _) = size().unwrap_or((80, 24));
    let max_inner = (term_cols as usize).saturating_sub(4).max(20);

    let path_display = truncate(&path.to_string_lossy(), max_inner.saturating_sub(2));
    // Content rows (without the box edges); menu rows reserve a 2-col cursor
    // gutter.
    let header_rows = 3; // title-row content, path, blank
    let mut rows: Vec<String> = vec![
        "Out-of-closure access:".to_string(),
        format!("  {path_display}"),
        String::new(),
    ];
    for item in menu {
        rows.push(format!(
            "  {}",
            truncate(&item.label, max_inner.saturating_sub(2))
        ));
    }

    let title = " Flox sandbox ";
    let inner = rows
        .iter()
        .map(|r| r.chars().count())
        .chain(std::iter::once(title.chars().count() + 1))
        .max()
        .unwrap_or(20)
        .min(max_inner);

    queue!(tty, Clear(ClearType::All), cursor::MoveTo(0, 0))?;

    // Top border with the embedded title.
    let dashes = inner
        .saturating_sub(title.chars().count())
        .saturating_sub(1);
    queue!(
        tty,
        Print(format!(
            "{TOP_LEFT}{HORIZONTAL}{title}{}{TOP_RIGHT}\r\n",
            HORIZONTAL.to_string().repeat(dashes)
        ))
    )?;

    for (i, row) in rows.iter().enumerate() {
        let menu_index = i.checked_sub(header_rows);
        let is_selected = menu_index == Some(selected);
        // The selected menu row gets the cursor marker and reverse video.
        let body = match menu_index {
            Some(_) if is_selected => format!("{CURSOR} {}", row.trim_start()),
            Some(_) => format!("  {}", row.trim_start()),
            None => row.clone(),
        };
        let padded = pad(&body, inner);
        queue!(tty, Print(format!("{VERTICAL} ")))?;
        if is_selected {
            queue!(
                tty,
                SetAttribute(Attribute::Reverse),
                Print(&padded),
                SetAttribute(Attribute::Reset)
            )?;
        } else {
            queue!(tty, Print(&padded))?;
        }
        queue!(tty, Print(format!(" {VERTICAL}\r\n")))?;
    }

    queue!(
        tty,
        Print(format!(
            "{BOTTOM_LEFT}{}{BOTTOM_RIGHT}\r\n",
            HORIZONTAL.to_string().repeat(inner + 2)
        )),
        Print(format!("{HINT}\r\n"))
    )?;
    tty.flush()
}

/// Pad `s` with spaces to exactly `width` display columns (truncating if longer).
fn pad(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        truncate(s, width)
    } else {
        format!("{s}{}", " ".repeat(width - len))
    }
}

/// Truncate `s` to at most `width` columns, ending with an ellipsis when
/// shortened.
fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let kept: String = s.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}{ELLIPSIS}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestor_globs_under_home_are_tilde_relative_and_stop_below_home() {
        let globs = ancestor_globs(
            Path::new("/home/u/.npm/_cacache/index-v5/a1/b2"),
            Some("/home/u"),
        );
        assert_eq!(globs, vec![
            "~/.npm/_cacache/index-v5/a1/**".to_string(),
            "~/.npm/_cacache/index-v5/**".to_string(),
            "~/.npm/_cacache/**".to_string(),
            "~/.npm/**".to_string(),
        ]);
        // Never offers all of $HOME.
        assert!(!globs.iter().any(|g| g == "~/**"));
    }

    #[test]
    fn ancestor_globs_outside_home_stop_at_root() {
        let globs = ancestor_globs(Path::new("/etc/hosts"), Some("/home/u"));
        assert_eq!(globs, vec!["/etc/**".to_string()]);
    }

    #[test]
    fn build_menu_has_allow_once_rollups_then_deny_choices() {
        let menu = build_menu(Path::new("/home/u/.npm/x"), Some("/home/u"));
        assert_eq!(menu.first().unwrap().choice, Choice::AllowOnce);
        assert_eq!(menu[menu.len() - 2].choice, Choice::Deny);
        assert_eq!(menu[menu.len() - 1].choice, Choice::DenyAll);
        assert_eq!(menu[1].choice, Choice::AllowGlob("~/.npm/**".to_string()));
    }

    #[test]
    fn decision_for_maps_choices() {
        let mut deny_all = false;
        assert_eq!(
            decision_for(Some(&Choice::AllowOnce), &mut deny_all),
            PromptDecision::Allow
        );
        assert_eq!(
            decision_for(Some(&Choice::AllowGlob("~/.npm/**".into())), &mut deny_all),
            PromptDecision::AllowGlob("~/.npm/**".into())
        );
        assert_eq!(
            decision_for(Some(&Choice::Deny), &mut deny_all),
            PromptDecision::Deny
        );
        assert_eq!(decision_for(None, &mut deny_all), PromptDecision::Deny);
        assert!(!deny_all);
        // Deny all flips the latch.
        assert_eq!(
            decision_for(Some(&Choice::DenyAll), &mut deny_all),
            PromptDecision::Deny
        );
        assert!(deny_all);
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("abcdef", 10), "abcdef");
        assert_eq!(truncate("abcdef", 4), format!("abc{ELLIPSIS}"));
        assert_eq!(truncate("abcdef", 0), "");
    }
}
