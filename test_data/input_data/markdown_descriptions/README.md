# Markdown description renderer comparison

Five `.flox` environments, one per Markdown construct that might render
differently across `FLOX_MARKDOWN_RENDERER`'s four backends
(`builtin` default, `mdcat`, `glow`, `gum`). Each has an empty
`[install]` table -- only the `description` field matters here.

| Directory | What it exercises |
|-----------|--------------------|
| `headings_and_emphasis` | `#`/`##` heading levels, bold, italic, strikethrough |
| `code_blocks` | inline code and a fenced block with a language tag (syntax highlighting) |
| `lists_quotes_links` | nested ordered/unordered lists, a block quote, a link |
| `table_and_wrapping` | a table (the builtin backend degrades it to plain text) and an 80-column-wrapping paragraph |
| `unicode_widths` | CJK wide characters, combining marks, and multi-codepoint emoji |

## Running the comparison

For each directory, run interactive activation under each backend and
compare the rendered description:

```
flox activate -d <manifest dir>
FLOX_MARKDOWN_RENDERER=mdcat flox activate -d <manifest dir>
FLOX_MARKDOWN_RENDERER=gum flox activate -d <manifest dir>
nix shell nixpkgs#glow -c env FLOX_MARKDOWN_RENDERER=glow flox activate -d <manifest dir>
```

`gum` (2.0.0) is in the dev shell. `glow` (3.0.0) is not -- it needs
`nix shell nixpkgs#glow`, and the backend degrades to plain text
without it.

## Open question this comparison should answer

Heading styling is deliberately unsettled. The builtin renderer bolds
and underlines every level the same way; one proposal is `#` bold and
`##` italic (colour alone was ruled out on accessibility grounds).
`gum format` takes a third approach: no glyph on H1, then "▌" and
"┃" prefixed onto H2 and H3. `headings_and_emphasis` is where to
compare the candidates side by side -- this file does not resolve it.
