//! Markdown-to-terminal rendering for the manifest `description` field.
//!
//! `render_markdown` turns a description into readable terminal text
//! instead of raw Markdown: a heading, `**bold**`, `*italic*`,
//! `~~struck~~`, and `` `code` `` all lose their syntax characters and
//! gain ANSI styling when stdout is a TTY. In plain mode (stdout not a
//! TTY) the styling is simply absent — a heading is its bare text, bold
//! text is its bare text — because `style_run` is the only place this
//! module adds characters that aren't part of the parsed content, and it
//! adds only escape codes, never markers. That makes `plain ==
//! strip_ansi(styled)` hold by construction for every construct, not
//! just the ones whose syntax happens to double as a readable plain-text
//! marker.
//!
//! Three constructs keep a literal marker in *both* modes, because the
//! marker there isn't leaked source syntax — it's the conventional
//! terminal rendering of that construct: list bullets and numbers
//! (`- `, `1. `), block-quote prefixes (`> `), and the four-space indent
//! that replaces a code fence. Links are a fourth exception with a
//! different plain rendering again: `text (url)`, per the design, not
//! `[text](url)`.
//!
//! Selected over `pulldown-cmark-mdcat` and a `glow` shell-out by a
//! comparison spike (ADR-2, `slices/2026/07-environment-readme/design.md`):
//! `pulldown-cmark` alone pulls in a handful of small, actively maintained
//! crates, while `pulldown-cmark-mdcat` drags in a syntax-highlighting and
//! image-rendering stack this single-field renderer doesn't need, and
//! `glow` requires bundling and shelling out to a multi-megabyte external
//! binary. A hand-rolled parser was rejected outright by that same ADR.
//!
//! `render_markdown` and `first_line_plain` are not yet called from any
//! command — wiring them into `flox activate` and `flox envs` is DEV-233
//! and DEV-232. `#![allow(dead_code)]` covers that gap deliberately rather
//! than the usual signal of an abandoned code path.
#![allow(dead_code)]

use flox_core::util::message::stdout_supports_color;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthChar;

/// Render `input` (a manifest `description`) for a terminal `width`.
///
/// ANSI styling is applied when stdout supports color; otherwise the
/// output is the same text with escape codes omitted. Never panics: a
/// `width` of zero, wide or combining characters, and words wider than
/// `width` are all handled by breaking output one display column at a
/// time rather than indexing by byte or `char` count. A parse that
/// produces no renderable text (for example, input that is only
/// whitespace after Markdown syntax is stripped) falls back to `input`
/// unchanged.
pub fn render_markdown(input: &str, width: usize) -> String {
    let rendered = render(input, width, stdout_supports_color());
    if rendered.is_empty() && !input.trim().is_empty() {
        return input.to_string();
    }
    rendered
}

/// Extract the first line of `input` as plain text, Markdown syntax
/// stripped.
///
/// A Setext H1 (a title line directly above a `===` underline) and an
/// ATX H1 (`# Title`) both parse to the same heading event, so both
/// yield the clean title with no extra handling here. Emphasis,
/// backticks, and link brackets are dropped; a link's URL is dropped
/// along with its brackets, keeping only the link text.
pub fn first_line_plain(input: &str) -> String {
    let parser = Parser::new(input);
    let mut result = String::new();
    let mut depth: i32 = 0;
    let mut started = false;
    let mut done = false;

    for event in parser {
        if done {
            break;
        }
        match event {
            Event::Start(Tag::Heading { .. } | Tag::Paragraph) if !started => {
                started = true;
                depth = 1;
            },
            Event::Start(_) if started => depth += 1,
            Event::End(_) if started => {
                depth -= 1;
                if depth <= 0 {
                    done = true;
                }
            },
            Event::Text(text) | Event::Code(text) if started => result.push_str(&text),
            Event::SoftBreak | Event::HardBreak if started => done = true,
            _ => {},
        }
    }

    let result = result.trim();
    if result.is_empty() && !input.trim().is_empty() {
        // No heading or paragraph parsed (e.g. the input is a bare code
        // block or table) — fall back to the first raw line rather than
        // silently returning nothing.
        return input.lines().next().unwrap_or_default().trim().to_string();
    }
    result.to_string()
}

/// The set of properties that can differ between runs of rendered text.
/// A run's ANSI escape sequence is derived from this; the styled and
/// plain renders always start from the same underlying characters, so
/// this struct is the *only* thing color mode is allowed to change.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    italic: bool,
    strikethrough: bool,
    code: bool,
    dim: bool,
    heading: bool,
}

/// Indent used for a fenced or indented code block line, in both modes.
const CODE_BLOCK_INDENT: &str = "    ";
/// Display columns a blockquote marker ("> ") occupies, per nesting level.
const BLOCKQUOTE_MARKER_WIDTH: usize = 2;

fn render(markdown: &str, width: usize, color: bool) -> String {
    let parser = Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH);

    let mut out: Vec<String> = Vec::new();
    let mut state = RenderState::new(width, color);

    for event in parser {
        state.handle(event, &mut out);
    }
    state.finish(&mut out);

    collapse_blank_lines(out)
}

/// One nested list's numbering/ordering context.
struct ListCtx {
    ordered: bool,
    next: u64,
}

/// The prefix an about-to-open block (paragraph, heading, or an item's
/// direct content) should use, queued by `Start(Item)` and consumed by
/// whichever block starts next inside it.
struct ItemPrefix {
    initial: String,
    hang: String,
    used: bool,
}

/// A block of inline content being accumulated between a block-level
/// `Start` and its matching `End`. `runs` holds one entry per hard break
/// inside the block (a fresh run per `HardBreak`), each a flat sequence
/// of characters carrying the style active when they were pushed.
struct ActiveBlock {
    initial_prefix: String,
    hang_prefix: String,
    /// Style applied to the prefix itself (e.g. bold+underlined for a
    /// heading's `#` marker) — the style active when the block began.
    prefix_style: Style,
    runs: Vec<Vec<(char, Style)>>,
}

impl ActiveBlock {
    fn new(initial_prefix: String, hang_prefix: String, prefix_style: Style) -> Self {
        Self {
            initial_prefix,
            hang_prefix,
            prefix_style,
            runs: vec![Vec::new()],
        }
    }

    fn push_str(&mut self, text: &str, style: Style) {
        let run = self.runs.last_mut().expect("runs is never empty");
        run.extend(text.chars().map(|c| (c, style)));
    }

    fn push_hard_break(&mut self) {
        self.runs.push(Vec::new());
    }
}

struct RenderState {
    width: usize,
    color: bool,
    active: Option<ActiveBlock>,
    list_stack: Vec<ListCtx>,
    item_prefix_stack: Vec<ItemPrefix>,
    blockquote_depth: usize,
    style_stack: Vec<InlineKind>,
    link_url: Option<String>,
    in_code_block: bool,
    code_buf: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineKind {
    Bold,
    Italic,
    Strikethrough,
    Code,
    Heading,
}

impl RenderState {
    fn new(width: usize, color: bool) -> Self {
        Self {
            width,
            color,
            active: None,
            list_stack: Vec::new(),
            item_prefix_stack: Vec::new(),
            blockquote_depth: 0,
            style_stack: Vec::new(),
            link_url: None,
            in_code_block: false,
            code_buf: None,
        }
    }

    fn current_style(&self) -> Style {
        Style {
            bold: self.style_stack.contains(&InlineKind::Bold),
            italic: self.style_stack.contains(&InlineKind::Italic),
            strikethrough: self.style_stack.contains(&InlineKind::Strikethrough),
            code: self.style_stack.contains(&InlineKind::Code),
            dim: false,
            heading: self.style_stack.contains(&InlineKind::Heading),
        }
    }

    /// Available width for wrapping content nested `blockquote_depth`
    /// levels deep. Saturates to zero rather than underflowing when the
    /// blockquote nesting alone exceeds the terminal width.
    fn content_width(&self) -> usize {
        self.width
            .saturating_sub(self.blockquote_depth * BLOCKQUOTE_MARKER_WIDTH)
    }

    fn flush_active(&mut self, out: &mut Vec<String>) {
        let Some(block) = self.active.take() else {
            return;
        };
        let content_width = self.content_width();
        let lines = wrap_block(&block, content_width, self.color);
        for line in lines {
            self.emit_line(out, line);
        }
    }

    fn emit_line(&self, out: &mut Vec<String>, line: String) {
        if self.blockquote_depth == 0 {
            out.push(line);
            return;
        }
        let marker = "> ".repeat(self.blockquote_depth);
        let marker = style_run(
            &marker,
            Style {
                dim: true,
                ..Style::default()
            },
            self.color,
        );
        out.push(format!("{marker}{line}"));
    }

    /// Blank-line separator between top-level blocks (paragraphs,
    /// headings, lists, block quotes, code blocks, rules). Suppressed
    /// while directly inside a list item, so list rendering stays tight
    /// rather than gaining a blank line after every marker.
    fn separator(&self, out: &mut Vec<String>) {
        if self.item_prefix_stack.is_empty() {
            out.push(String::new());
        }
    }

    /// Begin a new accumulating block (paragraph, heading, or an item's
    /// own direct text), consuming any queued item prefix.
    fn begin_block(&mut self, marker: Option<(String, String)>) {
        let (initial, hang) = marker.unwrap_or_else(|| {
            if let Some(item) = self.item_prefix_stack.last_mut() {
                let prefix = if item.used {
                    (item.hang.clone(), item.hang.clone())
                } else {
                    (item.initial.clone(), item.hang.clone())
                };
                item.used = true;
                prefix
            } else {
                (String::new(), String::new())
            }
        });
        self.active = Some(ActiveBlock::new(initial, hang, self.current_style()));
    }

    fn handle(&mut self, event: Event<'_>, out: &mut Vec<String>) {
        match event {
            Event::Start(tag) => self.start_tag(tag, out),
            Event::End(tag) => self.end_tag(tag, out),
            Event::Text(text) => self.push_text(&text, out),
            Event::Code(text) => {
                // The backticks are consumed as syntax, not carried into
                // the output — the code span is conveyed by the `code`
                // style (yellow in a TTY) rather than by punctuation, the
                // same as headings and emphasis.
                let style = Style {
                    code: true,
                    ..self.current_style()
                };
                self.ensure_active();
                if let Some(active) = self.active.as_mut() {
                    active.push_str(&text, style);
                }
            },
            Event::SoftBreak => self.push_text(" ", out),
            Event::HardBreak => {
                self.ensure_active();
                if let Some(active) = self.active.as_mut() {
                    active.push_hard_break();
                }
            },
            Event::Rule => {
                self.flush_active(out);
                self.separator(out);
                let rule = "-".repeat(self.content_width().max(1));
                let rule = style_run(
                    &rule,
                    Style {
                        dim: true,
                        ..Style::default()
                    },
                    self.color,
                );
                self.emit_line(out, rule);
                self.separator(out);
            },
            // Unsupported leaf constructs (math, raw inline HTML) degrade
            // to their plain text; footnote references and task-list
            // markers carry no useful plain-text form and are dropped.
            Event::InlineMath(text) | Event::DisplayMath(text) | Event::InlineHtml(text) => {
                self.push_text(&text, out);
            },
            Event::Html(_) | Event::FootnoteReference(_) | Event::TaskListMarker(_) => {},
        }
    }

    fn ensure_active(&mut self) {
        if self.active.is_none() {
            self.begin_block(None);
        }
    }

    fn push_text(&mut self, text: &str, out: &mut Vec<String>) {
        if self.in_code_block {
            self.push_code_block_text(text, out);
            return;
        }
        self.ensure_active();
        let style = self.current_style();
        if let Some(active) = self.active.as_mut() {
            active.push_str(text, style);
        }
    }

    /// A fenced code block's content can arrive as one multi-line `Text`
    /// event; each source line becomes one output line so wrapping never
    /// reflows code.
    fn push_code_block_text(&mut self, text: &str, out: &mut Vec<String>) {
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.emit_code_line(out);
            }
            self.code_buf.get_or_insert_with(String::new).push_str(line);
        }
    }

    fn emit_code_line(&mut self, out: &mut Vec<String>) {
        let line = self.code_buf.take().unwrap_or_default();
        let styled = style_run(
            &line,
            Style {
                code: true,
                ..Style::default()
            },
            self.color,
        );
        self.emit_line(out, format!("{CODE_BLOCK_INDENT}{styled}"));
    }

    fn start_tag(&mut self, tag: Tag<'_>, out: &mut Vec<String>) {
        match tag {
            Tag::Paragraph => {
                self.flush_active(out);
                self.separator(out);
                self.begin_block(None);
            },
            Tag::Heading { .. } => {
                self.flush_active(out);
                self.separator(out);
                self.style_stack.push(InlineKind::Heading);
                // Explicit empty prefix, not `None`: `None` would fall
                // back to any enclosing list item's marker (see
                // `begin_block`), and a heading nested in a list item
                // must not inherit the item's bullet as its own prefix.
                self.begin_block(Some((String::new(), String::new())));
            },
            Tag::BlockQuote(_) => {
                self.flush_active(out);
                self.separator(out);
                self.blockquote_depth += 1;
            },
            Tag::CodeBlock(_) => {
                self.flush_active(out);
                self.separator(out);
                self.in_code_block = true;
            },
            Tag::List(start) => {
                self.flush_active(out);
                self.separator(out);
                self.list_stack.push(ListCtx {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                });
            },
            Tag::Item => {
                self.flush_active(out);
                let depth_indent = "  ".repeat(self.list_stack.len().saturating_sub(1));
                let marker = match self.list_stack.last_mut() {
                    Some(ctx) if ctx.ordered => {
                        let n = ctx.next;
                        ctx.next += 1;
                        format!("{n}.")
                    },
                    _ => "-".to_string(),
                };
                let initial = format!("{depth_indent}{marker} ");
                let hang = " ".repeat(initial.chars().count());
                self.item_prefix_stack.push(ItemPrefix {
                    initial,
                    hang,
                    used: false,
                });
            },
            Tag::Emphasis => self.open_inline(InlineKind::Italic),
            Tag::Strong => self.open_inline(InlineKind::Bold),
            Tag::Strikethrough => self.open_inline(InlineKind::Strikethrough),
            Tag::Link { dest_url, .. } => {
                self.link_url = Some(dest_url.into_string());
            },
            // Unsupported container constructs (tables, footnote
            // definitions, images, definition lists, metadata,
            // superscript/subscript, HTML blocks) degrade to their
            // inner plain text: no special handling here means their
            // nested `Text` events fall through to `push_text` with
            // whatever style is already active.
            _ => {},
        }
    }

    /// Emphasis, strong, and strikethrough carry no marker of their own
    /// in the output — `**`/`*`/`~~` are consumed as syntax, same as a
    /// heading's `#`. The construct is conveyed entirely by `style`,
    /// applied to whatever text arrives while `kind` is on the stack.
    fn open_inline(&mut self, kind: InlineKind) {
        self.style_stack.push(kind);
    }

    fn close_inline(&mut self, kind: InlineKind) {
        if let Some(pos) = self.style_stack.iter().rposition(|k| *k == kind) {
            self.style_stack.remove(pos);
        }
    }

    fn end_tag(&mut self, tag: TagEnd, out: &mut Vec<String>) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_active(out);
                self.separator(out);
            },
            TagEnd::Heading(_) => {
                self.flush_active(out);
                if let Some(pos) = self
                    .style_stack
                    .iter()
                    .rposition(|k| *k == InlineKind::Heading)
                {
                    self.style_stack.remove(pos);
                }
                self.separator(out);
            },
            TagEnd::BlockQuote(_) => {
                self.flush_active(out);
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
                self.separator(out);
            },
            TagEnd::CodeBlock => {
                if self.code_buf.is_some() {
                    self.emit_code_line(out);
                }
                self.in_code_block = false;
                self.separator(out);
            },
            TagEnd::List(_) => {
                self.list_stack.pop();
                self.separator(out);
            },
            TagEnd::Item => {
                self.flush_active(out);
                self.item_prefix_stack.pop();
            },
            TagEnd::Emphasis => self.close_inline(InlineKind::Italic),
            TagEnd::Strong => self.close_inline(InlineKind::Bold),
            TagEnd::Strikethrough => self.close_inline(InlineKind::Strikethrough),
            TagEnd::Link => {
                let url = self.link_url.take().unwrap_or_default();
                let style = Style {
                    dim: true,
                    ..Style::default()
                };
                self.ensure_active();
                if let Some(active) = self.active.as_mut() {
                    active.push_str(" (", style);
                    active.push_str(&url, style);
                    active.push_str(")", style);
                }
            },
            _ => {},
        }
    }

    fn finish(&mut self, out: &mut Vec<String>) {
        self.flush_active(out);
        if self.code_buf.is_some() {
            self.emit_code_line(out);
        }
    }
}

/// Wrap `block`'s accumulated runs to `width` display columns, returning
/// one string per output line (prefix included, styled if `color`).
fn wrap_block(block: &ActiveBlock, width: usize, color: bool) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (i, run) in block.runs.iter().enumerate() {
        let words = split_words(run);
        let mut run_lines = wrap_words(
            &words,
            width,
            &block.initial_prefix,
            &block.hang_prefix,
            block.prefix_style,
            color,
            i == 0,
        );
        lines.append(&mut run_lines);
    }
    if lines.is_empty() && !block.initial_prefix.is_empty() {
        // An empty block (e.g. an empty list item) still gets its marker.
        lines.push(style_prefix_raw(
            block.initial_prefix.trim_end(),
            block.prefix_style,
            color,
        ));
    }
    lines
}

/// Split a flat character run into words (whitespace-delimited, styled
/// runs kept intact), dropping the whitespace itself — wrapped output
/// always rejoins words with a single space.
fn split_words(run: &[(char, Style)]) -> Vec<Vec<(char, Style)>> {
    let mut words = Vec::new();
    let mut current = Vec::new();
    for &(c, style) in run {
        if c.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push((c, style));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// The on-screen width of a single character, treating anything
/// `unicode-width` can't size (control characters) as zero columns
/// rather than panicking or guessing.
fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

fn word_width(word: &[(char, Style)]) -> usize {
    word.iter().map(|(c, _)| char_width(*c)).sum()
}

/// Greedily wrap `words` to `width` display columns, using
/// `initial_prefix` on the first output line this call produces and
/// `hang_prefix` on every line after — `first_run` distinguishes "first
/// line of the whole block" from "first line of a run after a hard
/// break", since only the former gets the marker.
fn wrap_words(
    words: &[Vec<(char, Style)>],
    width: usize,
    initial_prefix: &str,
    hang_prefix: &str,
    prefix_style: Style,
    color: bool,
    first_run: bool,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current: Vec<&[(char, Style)]> = Vec::new();
    let mut current_width = 0usize;

    let prefix_for = |lines_so_far: usize| -> &str {
        if lines_so_far == 0 && first_run {
            initial_prefix
        } else {
            hang_prefix
        }
    };
    let prefix_width = |p: &str| -> usize { p.chars().map(char_width).sum() };

    for word in words {
        let ww = word_width(word);
        let avail = width.saturating_sub(prefix_width(prefix_for(lines.len())));
        let sep = usize::from(!current.is_empty());

        if !current.is_empty() && current_width + sep + ww > avail {
            lines.push(render_line(
                &current,
                prefix_for(lines.len()),
                prefix_style,
                color,
            ));
            current.clear();
            current_width = 0;
        }

        let avail = width.saturating_sub(prefix_width(prefix_for(lines.len())));
        if ww > avail.max(1) {
            // A single word wider than the available width: hard-break
            // it one display column at a time rather than overflowing.
            let mut remaining: &[(char, Style)] = word;
            while !remaining.is_empty() {
                let avail = width
                    .saturating_sub(prefix_width(prefix_for(lines.len())))
                    .max(1);
                let split_at = split_index_by_width(remaining, avail);
                let (chunk, rest) = remaining.split_at(split_at);
                lines.push(render_line(
                    &[chunk],
                    prefix_for(lines.len()),
                    prefix_style,
                    color,
                ));
                remaining = rest;
            }
            continue;
        }

        current.push(word);
        current_width += sep + ww;
    }
    if !current.is_empty() {
        lines.push(render_line(
            &current,
            prefix_for(lines.len()),
            prefix_style,
            color,
        ));
    }
    lines
}

fn render_line(
    words: &[&[(char, Style)]],
    prefix: &str,
    prefix_style: Style,
    color: bool,
) -> String {
    let mut line = style_prefix_raw(prefix, prefix_style, color);
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        line.push_str(&render_run(word, color));
    }
    line
}

fn style_prefix_raw(prefix: &str, prefix_style: Style, color: bool) -> String {
    if prefix.is_empty() {
        return String::new();
    }
    style_run(prefix, prefix_style, color)
}

/// Find the char index in `chars` where the cumulative display width
/// first exceeds `avail`. Always returns at least `1` when `chars` is
/// non-empty, guaranteeing forward progress even when `avail` is `0`
/// (e.g. a caller-supplied `width` of zero) — the alternative is an
/// infinite loop in the hard-break path above.
fn split_index_by_width(chars: &[(char, Style)], avail: usize) -> usize {
    let mut used = 0usize;
    let mut idx = 0usize;
    for (i, (c, _)) in chars.iter().enumerate() {
        let w = char_width(*c);
        if idx > 0 && used + w > avail {
            break;
        }
        used += w;
        idx = i + 1;
    }
    idx.max(1).min(chars.len())
}

/// Render a styled character run, batching consecutive characters that
/// share a style into one ANSI-wrapped chunk.
fn render_run(chars: &[(char, Style)], color: bool) -> String {
    let mut out = String::new();
    let mut batch = String::new();
    let mut batch_style = Style::default();
    let mut have_batch = false;

    for &(c, style) in chars {
        if have_batch && style != batch_style {
            out.push_str(&style_run(&batch, batch_style, color));
            batch.clear();
        }
        batch.push(c);
        batch_style = style;
        have_batch = true;
    }
    if have_batch {
        out.push_str(&style_run(&batch, batch_style, color));
    }
    out
}

/// Apply `style` as ANSI escapes when `color` is set; otherwise return
/// `text` unchanged. This is the only place ANSI codes are added, so
/// `render(md, width, false)` is always `strip_ansi(render(md, width,
/// true))` for the same input.
fn style_run(text: &str, style: Style, color: bool) -> String {
    if !color || text.is_empty() || style == Style::default() {
        return text.to_string();
    }
    use crossterm::style::Stylize;
    let mut styled = text.stylize();
    if style.bold {
        styled = styled.bold();
    }
    if style.italic {
        styled = styled.italic();
    }
    if style.strikethrough {
        styled = styled.crossed_out();
    }
    if style.code {
        styled = styled.yellow();
    }
    if style.dim {
        styled = styled.dark_grey();
    }
    if style.heading {
        styled = styled.bold().underlined();
    }
    styled.to_string()
}

/// Collapse runs of blank lines to one and trim leading/trailing blanks,
/// mirroring how block-level Markdown elements are conventionally
/// separated by a single blank line regardless of source spacing.
fn collapse_blank_lines(lines: Vec<String>) -> String {
    let mut rendered = String::new();
    let mut prev_blank = true;
    for line in lines {
        let is_blank = line.trim().is_empty();
        if is_blank && prev_blank {
            continue;
        }
        rendered.push_str(&line);
        rendered.push('\n');
        prev_blank = is_blank;
    }
    rendered.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn strip_ansi(s: &str) -> String {
        // Matches the SGR sequences this module emits (`\x1b[...m`); good
        // enough for round-tripping test output without pulling in a
        // regex dependency just for tests.
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' && chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
                continue;
            }
            out.push(c);
        }
        out
    }

    fn styled(md: &str, width: usize) -> String {
        render(md, width, true)
    }

    fn plain(md: &str, width: usize) -> String {
        render(md, width, false)
    }

    #[test]
    fn plain_mode_is_styled_mode_minus_escape_codes() {
        let md = indoc::indoc! {"
            # Heading

            A **bold** and *italic* and ~~struck~~ paragraph with `code`
            and a [link](https://example.com).

            - one
            - two

            1. first
            2. second

            > a quote

            ```
            fn main() {}
            ```
        "};
        for width in [0, 1, 10, 20, 80] {
            let styled_out = styled(md, width);
            let plain_out = plain(md, width);
            assert_eq!(strip_ansi(&styled_out), plain_out, "width={width}");
        }
    }

    #[test]
    fn heading_drops_atx_marker_and_bolds_in_color() {
        assert_eq!(plain("# Title", 80), "Title");
        assert!(styled("# Title", 80).contains("\x1b["));
    }

    #[test]
    fn bold_italic_strikethrough_drop_markdown_markers_and_style_in_color() {
        assert_eq!(plain("**bold**", 80), "bold");
        assert_eq!(plain("*italic*", 80), "italic");
        assert_eq!(plain("~~struck~~", 80), "struck");
        assert!(styled("**bold**", 80).contains("\x1b["));
    }

    #[test]
    fn inline_code_content_is_never_reinterpreted_but_backticks_are_dropped() {
        // "**not bold**" survives literally — the code span protects it
        // from being reparsed as emphasis — but the backticks that mark
        // it as code are consumed as syntax, same as heading/emphasis
        // markers, and conveyed by color instead.
        assert_eq!(plain("`**not bold**`", 80), "**not bold**");
        assert!(styled("`**not bold**`", 80).contains("\x1b["));
    }

    #[test]
    fn fenced_code_block_is_indented_and_never_reinterpreted() {
        // Fenced code keeps its four-space indent in both modes (a
        // conventional terminal rendering, not leaked syntax) and its
        // content is never reparsed as Markdown.
        let md = "```\nlet x = **not bold**;\n```";
        assert_eq!(plain(md, 80), "    let x = **not bold**;");
    }

    #[test]
    fn unordered_and_ordered_lists_render_markers() {
        let md = "- one\n- two\n";
        assert_eq!(plain(md, 80), "- one\n- two");

        let md = "1. first\n2. second\n";
        assert_eq!(plain(md, 80), "1. first\n2. second");
    }

    #[test]
    fn nested_list_items_indent_and_continue_numbering() {
        let md = "1. outer\n   - inner one\n   - inner two\n2. outer two\n";
        assert_eq!(
            plain(md, 80),
            "1. outer\n  - inner one\n  - inner two\n2. outer two"
        );
    }

    #[test]
    fn block_quote_prefixes_every_line() {
        let md = "> a quote that is long enough to wrap onto a second line of output";
        let out = plain(md, 20);
        assert!(out.lines().all(|l| l.starts_with("> ")), "{out:?}");
    }

    #[test]
    fn link_renders_text_then_url_no_osc8() {
        let out = plain("See [the docs](https://flox.dev/docs).", 80);
        assert_eq!(out, "See the docs (https://flox.dev/docs).");
        assert!(!out.contains('\x1b'));
    }

    #[test]
    fn unsupported_table_construct_degrades_to_plain_text() {
        let md = "| a | b |\n| - | - |\n| c | d |\n";
        let out = plain(md, 80);
        // No panic, and the cell text survives even without table
        // formatting.
        assert!(out.contains('a') && out.contains('b'));
        assert!(out.contains('c') && out.contains('d'));
    }

    #[test]
    fn zero_width_never_panics_and_makes_progress() {
        let md = "a paragraph of ordinary words that must still terminate";
        let out = render_markdown(md, 0);
        assert!(!out.is_empty());
    }

    #[test]
    fn wide_and_combining_characters_never_panic() {
        // CJK wide characters, a zero-width joiner emoji sequence, and a
        // combining mark, each exercised at a tight width so the
        // truncation boundary lands mid-cluster.
        for md in [
            "宽字符宽字符宽字符宽字符宽字符宽字符宽字符",
            "family emoji: 👨‍👩‍👧‍👦 end",
            "e\u{0301}e\u{0301}e\u{0301} combining marks",
        ] {
            for width in [0, 1, 2, 3, 5] {
                let out = render_markdown(md, width);
                assert!(!out.is_empty(), "md={md:?} width={width}");
            }
        }
    }

    #[test]
    fn non_tty_output_has_no_escape_codes() {
        let out = plain("**bold** and *italic*", 80);
        assert!(!out.contains('\x1b'));
    }

    #[test]
    fn empty_input_renders_to_empty_string() {
        assert_eq!(render_markdown("", 80), "");
    }

    #[test]
    fn unparseable_input_falls_back_to_raw_string() {
        // A lone backslash has no block-level content pulldown-cmark
        // considers renderable text on its own escape target, so this
        // exercises the "parse yields nothing" fallback path directly
        // rather than asserting on parser internals.
        let out = render_markdown("\\", 80);
        assert_eq!(out, "\\");
    }

    #[test]
    fn first_line_plain_strips_atx_h1_marker() {
        assert_eq!(
            first_line_plain("# My Environment\n\nBody text."),
            "My Environment"
        );
    }

    #[test]
    fn first_line_plain_strips_setext_h1_underline() {
        assert_eq!(
            first_line_plain("My Environment\n===============\n\nBody text."),
            "My Environment"
        );
    }

    #[test]
    fn first_line_plain_strips_inline_syntax() {
        assert_eq!(
            first_line_plain("A **bold** and `code` and [a link](https://x) summary."),
            "A bold and code and a link summary."
        );
    }

    #[test]
    fn first_line_plain_takes_only_first_line_of_paragraph() {
        assert_eq!(
            first_line_plain("First line of the paragraph\nSecond line."),
            "First line of the paragraph"
        );
    }

    #[test]
    fn first_line_plain_of_empty_input_is_empty() {
        assert_eq!(first_line_plain(""), "");
    }
}
