//! A small Markdown-to-terminal renderer.
//!
//! Flox environments document themselves with a `README.md` that is rendered in
//! the terminal by `flox info` and (optionally) on `flox activate`. Rather than
//! pull in a heavyweight rendering dependency, this module handles the subset of
//! Markdown that README files typically use: headings, paragraphs, fenced and
//! inline code, bold/italic emphasis, links, blockquotes, bullet and numbered
//! lists, horizontal rules, and pipe tables.
//!
//! Styling is applied with `crossterm` and degrades to plain (but still
//! structured) text when the output stream does not support color, e.g. when
//! piped to a file.
use std::sync::LazyLock;

use crossterm::style::Stylize;
use regex::{Captures, Regex};
use textwrap::{Options, fill};

/// Matches the inline constructs we style, in priority order: code spans first
/// (their contents are never reinterpreted), then links, then bold, then
/// italic. The whole match is scanned in a single pass so replacements are
/// never re-examined.
static INLINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?P<code>`[^`]+`)
        | (?P<link>\[[^\]]+\]\([^)]+\))
        | (?P<bold>\*\*[^*]+\*\*|__[^_]+__)
        | (?P<italic>\*[^*]+\*|_[^_]+_)
        ",
    )
    .expect("inline markdown regex is valid")
});

static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.*?)\s*#*\s*$").expect("heading regex is valid"));
static BULLET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)[-*+]\s+(.*)$").expect("bullet regex is valid"));
static ORDERED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)(\d+)[.)]\s+(.*)$").expect("ordered regex is valid"));
static FENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(```|~~~)").expect("fence regex is valid"));
// The `regex` crate has no backreferences, so each rule character is matched by
// its own alternative rather than `([-*_])(\s*\1){2,}`.
static RULE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:-[ \t]*){3,}$|^\s*(?:\*[ \t]*){3,}$|^\s*(?:_[ \t]*){3,}$")
        .expect("rule regex is valid")
});
static TABLE_ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\|.*\|\s*$").expect("table row regex is valid"));

/// Render `markdown` into styled, wrapped text fit for a terminal `width`.
///
/// When `color` is false, the same structure is produced without ANSI escapes
/// (headings are upper-cased, code is fenced with indentation, and so on) so the
/// output stays readable when redirected.
pub fn render(markdown: &str, width: usize, color: bool) -> String {
    let pen = Pen { color };
    // Leave a little breathing room and never wrap absurdly wide.
    let width = width.clamp(20, 100);
    let mut out: Vec<String> = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut table_rows: Vec<String> = Vec::new();
    let mut in_code_block = false;

    let flush_paragraph = |paragraph: &mut Vec<String>, out: &mut Vec<String>| {
        if paragraph.is_empty() {
            return;
        }
        let joined = paragraph.join(" ");
        let styled = format_inline(&joined, &pen);
        out.push(fill(&styled, wrap_options(width)));
        paragraph.clear();
    };
    let flush_table = |table_rows: &mut Vec<String>, out: &mut Vec<String>| {
        if table_rows.is_empty() {
            return;
        }
        out.extend(render_table(table_rows, &pen));
        table_rows.clear();
    };

    for line in markdown.lines() {
        if FENCE.is_match(line) {
            flush_paragraph(&mut paragraph, &mut out);
            flush_table(&mut table_rows, &mut out);
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            out.push(format!("    {}", pen.code_block(line)));
            continue;
        }

        if TABLE_ROW.is_match(line) {
            flush_paragraph(&mut paragraph, &mut out);
            table_rows.push(line.to_string());
            continue;
        }
        flush_table(&mut table_rows, &mut out);

        let trimmed = line.trim();

        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut out);
            out.push(String::new());
            continue;
        }

        if RULE.is_match(line) {
            flush_paragraph(&mut paragraph, &mut out);
            out.push(pen.rule(width));
            continue;
        }

        if let Some(caps) = HEADING.captures(line) {
            flush_paragraph(&mut paragraph, &mut out);
            let level = caps[1].len();
            let text = format_inline(&caps[2], &pen);
            out.push(pen.heading(level, &text));
            continue;
        }

        if let Some(caps) = BULLET.captures(line) {
            flush_paragraph(&mut paragraph, &mut out);
            let indent = caps[1].len();
            let text = format_inline(&caps[2], &pen);
            out.push(render_list_item(indent, &pen.bullet(), &text, width));
            continue;
        }

        if let Some(caps) = ORDERED.captures(line) {
            flush_paragraph(&mut paragraph, &mut out);
            let indent = caps[1].len();
            let marker = pen.ordered_marker(&caps[2]);
            let text = format_inline(&caps[3], &pen);
            out.push(render_list_item(indent, &marker, &text, width));
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('>') {
            flush_paragraph(&mut paragraph, &mut out);
            let text = format_inline(rest.trim(), &pen);
            out.push(format!("{} {}", pen.quote_bar(), pen.quote_text(&text)));
            continue;
        }

        paragraph.push(trimmed.to_string());
    }
    flush_paragraph(&mut paragraph, &mut out);
    flush_table(&mut table_rows, &mut out);

    // Collapse runs of blank lines and trim leading/trailing blanks.
    let mut rendered = String::new();
    let mut prev_blank = true;
    for line in out {
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

/// Render a list item, hanging-indenting wrapped continuation lines under the
/// text so they line up past the marker.
fn render_list_item(indent: usize, marker: &str, text: &str, width: usize) -> String {
    let lead = " ".repeat(indent);
    let initial = format!("{lead}{marker} ");
    // The visible marker width (the marker may contain ANSI codes), measured
    // from the plain lead plus a bullet/number and a space.
    let hang = " ".repeat(initial_visible_len(indent, marker));
    let options = wrap_options(width)
        .initial_indent(&initial)
        .subsequent_indent(&hang);
    fill(text, options)
}

fn initial_visible_len(indent: usize, marker: &str) -> usize {
    indent + visible_len(marker) + 1
}

/// The on-screen width of `text`, ignoring ANSI escapes.
fn visible_len(text: &str) -> usize {
    static ANSI: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*m").expect("ansi regex is valid"));
    ANSI.replace_all(text, "").chars().count()
}

/// Render a pipe table as columns padded to their widest cell. The alignment
/// separator row (`| --- | :-: |`) marks the row above it as a header, which is
/// emphasized and underlined; alignment itself is not honored.
fn render_table(rows: &[String], pen: &Pen) -> Vec<String> {
    let mut header: Option<Vec<String>> = None;
    let mut body: Vec<Vec<String>> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let cells: Vec<&str> = row
            .trim()
            .trim_start_matches('|')
            .trim_end_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let is_separator = cells
            .iter()
            .all(|c| c.contains('-') && c.chars().all(|ch| matches!(ch, '-' | ':')));
        if is_separator {
            if i == 1 && body.len() == 1 {
                header = body.pop();
            }
            continue;
        }
        body.push(cells.iter().map(|c| format_inline(c, pen)).collect());
    }

    let columns = header
        .iter()
        .chain(body.iter())
        .map(Vec::len)
        .max()
        .unwrap_or(0);
    let mut widths = vec![0usize; columns];
    for row in header.iter().chain(body.iter()) {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(visible_len(cell));
        }
    }

    let pad_row = |row: &[String]| -> String {
        let mut line = String::new();
        for (i, width) in widths.iter().enumerate() {
            let cell = row.get(i).map(String::as_str).unwrap_or("");
            line.push_str(cell);
            if i + 1 < columns {
                line.push_str(&" ".repeat(width - visible_len(cell) + 2));
            }
        }
        line.trim_end().to_string()
    };

    let mut out = Vec::new();
    if let Some(header) = header {
        let emphasized: Vec<String> = header.iter().map(|c| pen.bold(c)).collect();
        out.push(pad_row(&emphasized));
        out.push(
            widths
                .iter()
                .map(|w| pen.table_rule(*w))
                .collect::<Vec<_>>()
                .join("  "),
        );
    }
    out.extend(body.iter().map(|row| pad_row(row)));
    out
}

fn wrap_options(width: usize) -> Options<'static> {
    Options::new(width)
        .break_words(false)
        .word_splitter(textwrap::WordSplitter::NoHyphenation)
}

/// Apply inline styling (code, links, bold, italic) in a single pass.
fn format_inline(text: &str, pen: &Pen) -> String {
    INLINE
        .replace_all(text, |caps: &Captures| {
            if let Some(m) = caps.name("code") {
                return pen.code_span(m.as_str().trim_matches('`'));
            }
            if let Some(m) = caps.name("link") {
                return pen.link(m.as_str());
            }
            if let Some(m) = caps.name("bold") {
                let inner = m.as_str().trim_matches('*').trim_matches('_');
                return pen.bold(inner);
            }
            if let Some(m) = caps.name("italic") {
                let inner = m.as_str().trim_matches('*').trim_matches('_');
                return pen.italic(inner);
            }
            caps[0].to_string()
        })
        .into_owned()
}

/// Styling helper that emits ANSI when `color` is set and plain text otherwise.
struct Pen {
    color: bool,
}

impl Pen {
    fn heading(&self, level: usize, text: &str) -> String {
        if !self.color {
            return match level {
                1 => format!(
                    "{}\n{}",
                    text.to_uppercase(),
                    "=".repeat(text.chars().count())
                ),
                2 => format!("{}\n{}", text, "-".repeat(text.chars().count())),
                _ => format!("{} {text}", "#".repeat(level)),
            };
        }
        match level {
            1 => text.to_string().magenta().bold().underlined().to_string(),
            2 => text.to_string().cyan().bold().to_string(),
            _ => text.to_string().bold().to_string(),
        }
    }

    fn code_span(&self, text: &str) -> String {
        if self.color {
            text.to_string().yellow().to_string()
        } else {
            format!("`{text}`")
        }
    }

    fn code_block(&self, text: &str) -> String {
        if self.color {
            text.to_string().grey().to_string()
        } else {
            text.to_string()
        }
    }

    fn bold(&self, text: &str) -> String {
        if self.color {
            text.to_string().bold().to_string()
        } else {
            text.to_uppercase()
        }
    }

    fn italic(&self, text: &str) -> String {
        if self.color {
            text.to_string().italic().to_string()
        } else {
            text.to_string()
        }
    }

    fn link(&self, raw: &str) -> String {
        // raw is `[text](url)`.
        let inner = &raw[1..];
        let Some(idx) = inner.find("](") else {
            return raw.to_string();
        };
        let text = &inner[..idx];
        let url = &inner[idx + 2..inner.len() - 1];
        if self.color {
            format!(
                "{} ({})",
                text.to_string().cyan().underlined(),
                url.to_string().dark_grey()
            )
        } else {
            format!("{text} ({url})")
        }
    }

    fn bullet(&self) -> String {
        if self.color {
            "•".to_string().cyan().to_string()
        } else {
            "-".to_string()
        }
    }

    fn ordered_marker(&self, number: &str) -> String {
        let marker = format!("{number}.");
        if self.color {
            marker.cyan().to_string()
        } else {
            marker
        }
    }

    fn quote_bar(&self) -> String {
        if self.color {
            "│".to_string().dark_grey().to_string()
        } else {
            ">".to_string()
        }
    }

    fn quote_text(&self, text: &str) -> String {
        if self.color {
            text.to_string().italic().grey().to_string()
        } else {
            text.to_string()
        }
    }

    fn rule(&self, width: usize) -> String {
        let line = "─".repeat(width);
        if self.color {
            line.dark_grey().to_string()
        } else {
            "-".repeat(width)
        }
    }

    fn table_rule(&self, width: usize) -> String {
        if self.color {
            "─".repeat(width).dark_grey().to_string()
        } else {
            "-".repeat(width)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_render_preserves_structure() {
        let md = "# Title\n\nSome **bold** and `code`.\n\n- one\n- two\n";
        let out = render(md, 80, false);
        assert!(out.contains("TITLE"));
        assert!(out.contains("BOLD"));
        assert!(out.contains("`code`"));
        assert!(out.contains("- one"));
        assert!(out.contains("- two"));
    }

    #[test]
    fn code_blocks_are_indented_and_not_formatted() {
        let md = "```\nlet x = **not bold**;\n```\n";
        let out = render(md, 80, false);
        assert!(out.contains("    let x = **not bold**;"));
    }

    #[test]
    fn links_show_text_and_url_in_plain_mode() {
        let md = "See [the docs](https://flox.dev/docs).";
        let out = render(md, 80, false);
        assert!(out.contains("the docs (https://flox.dev/docs)"));
    }

    #[test]
    fn color_render_emits_ansi_for_headings() {
        let out = render("# Title", 80, true);
        assert!(out.contains("\x1b["), "expected ANSI escape codes: {out:?}");
    }

    #[test]
    fn ordered_lists_keep_numbers() {
        let md = "1. first\n2. second\n";
        let out = render(md, 80, false);
        assert!(out.contains("1. first"));
        assert!(out.contains("2. second"));
    }

    #[test]
    fn tables_align_columns_and_underline_header() {
        let md = indoc::indoc! {"
            | Command | What it does |
            | ------- | ------------ |
            | `flox info` | Show the README |
            | `flox activate` | Enter the environment |
        "};
        let out = render(md, 80, false);
        assert_eq!(out, indoc::indoc! {"
            COMMAND          WHAT IT DOES
            ---------------  ---------------------
            `flox info`      Show the README
            `flox activate`  Enter the environment"
        });
    }

    #[test]
    fn table_without_separator_has_no_header() {
        let md = "| a | b |\n| c | d |\n";
        let out = render(md, 80, false);
        assert_eq!(out, "a  b\nc  d");
    }
}
