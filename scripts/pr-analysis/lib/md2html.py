"""Pure-stdlib Markdown → HTML rendering for the PR-analysis artifacts.

Handles the subset of CommonMark needed by the artifacts produced in
`scripts/pr-analysis/`:

- ATX headers (`#`, `##`, `###`, `####`, `#####`, `######`)
- Unordered bullets (`-`, `*`, `+`) including nesting by 2-space indent
- Ordered lists (`1.`, `2.`, …)
- Fenced code blocks (triple-backtick, optional language hint)
- Inline code (single backtick)
- Bold (`**…**`), italic (`*…*` / `_…_`)
- Links `[text](url)`
- Horizontal rules (`---`, `***`, `___`)
- Pipe-style tables with `---|---` separator row
- Blockquotes (`> `)
- Paragraphs (blank-line separated)

The output is a self-contained HTML document using the same CSS palette as
`rust-pr-analysis-dashboard-01.html` (kept in sync via the constant in this
module). The caller passes:

- `markdown_text`: raw source.
- `title`: title used in `<head><title>` and in the H1 of the page header.
- `subtitle_html`: pre-rendered HTML for the subtitle line (the caller knows
  what's a code span and what's a date, so it composes this).
- `back_href`: relative URL to the master index page. Compute it via
  `os.path.relpath(INDEX, html_path.parent)` at the call site.
- `source_path_repo`: repo-relative source path used in the "Source:" line.
- `include_toc`: when True, scan the rendered document for h2/h3 anchors and
  emit a table of contents at the top. Useful for very long files.

Nothing in this module touches the network or the database.
"""
from __future__ import annotations

import datetime as dt
import html
import re
from dataclasses import dataclass, field
from typing import Iterable

# Shared dashboard-palette CSS, plus md2html-specific tweaks for tables, lists,
# blockquotes, and the TOC nav block.
CSS = """
:root {
  --fg: #1c1f24; --fg-mute: #5a6270; --bg: #fbfbfa; --bg-card: #ffffff;
  --border: #e4e6eb; --accent: #3b6bd6; --good: #2f8a52; --warn: #c98a17;
  --bad: #c44545; --neutral: #6c757d; --code-bg: #f3f4f6;
  --shadow: 0 1px 2px rgba(0,0,0,.04), 0 4px 12px rgba(0,0,0,.04);
}
* { box-sizing: border-box; }
body {
  font: 15px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
  color: var(--fg); background: var(--bg); margin: 0; padding: 32px 24px 80px;
}
.container { max-width: 1080px; margin: 0 auto; }
header.page-header {
  background: var(--bg-card); border: 1px solid var(--border);
  border-radius: 8px; padding: 24px 28px; box-shadow: var(--shadow);
  margin-bottom: 22px;
}
header.page-header h1 { margin: 0 0 4px; font-size: 26px; font-weight: 600; letter-spacing: -.01em; }
header.page-header .subtitle { color: var(--fg-mute); font-size: 14px; margin: 0; }
header.page-header .back { margin-top: 14px; font-size: 13px; }
header.page-header .back a { color: var(--accent); text-decoration: none; }
header.page-header .back a:hover { text-decoration: underline; }
section.body {
  background: var(--bg-card); border: 1px solid var(--border);
  border-radius: 8px; padding: 24px 32px; box-shadow: var(--shadow);
}
section.body h1 { font-size: 22px; font-weight: 600; margin: 22px 0 10px; letter-spacing: -.005em; border-bottom: 1px solid var(--border); padding-bottom: 4px; }
section.body h2 { font-size: 19px; font-weight: 600; margin: 26px 0 10px; letter-spacing: -.005em; border-bottom: 1px solid var(--border); padding-bottom: 4px; }
section.body h2:first-child, section.body h1:first-child { margin-top: 0; }
section.body h3 { font-size: 16px; font-weight: 600; margin: 20px 0 8px; }
section.body h4 { font-size: 14px; font-weight: 600; margin: 16px 0 6px; color: var(--fg-mute); text-transform: uppercase; letter-spacing: .03em; }
section.body h5, section.body h6 { font-size: 13px; font-weight: 600; margin: 14px 0 4px; }
section.body p { margin: 0 0 12px; }
section.body ul, section.body ol { margin: 6px 0 12px; padding-left: 26px; }
section.body li { margin: 3px 0; }
section.body li > p { margin: 0 0 6px; }
section.body code {
  font-family: "SF Mono", Menlo, Consolas, monospace; font-size: 13px;
  background: var(--code-bg); padding: 1px 5px; border-radius: 3px;
}
section.body pre {
  font-family: "SF Mono", Menlo, Consolas, monospace; font-size: 12.5px;
  background: var(--code-bg); border-radius: 6px; padding: 14px 16px;
  overflow-x: auto; line-height: 1.45; margin: 8px 0 14px;
}
section.body pre code { background: transparent; padding: 0; font-size: 12.5px; }
section.body blockquote {
  border-left: 3px solid var(--border); padding: 6px 14px; margin: 10px 0;
  background: var(--code-bg); color: var(--fg-mute); border-radius: 0 6px 6px 0;
}
section.body blockquote p { margin: 4px 0; }
section.body a { color: var(--accent); text-decoration: none; }
section.body a:hover { text-decoration: underline; }
section.body hr { border: none; border-top: 1px solid var(--border); margin: 22px 0; }
section.body table {
  width: 100%; border-collapse: collapse; margin: 8px 0 16px; font-size: 13.5px;
}
section.body th, section.body td {
  border: 1px solid var(--border); padding: 7px 10px; text-align: left; vertical-align: top;
}
section.body th { background: var(--code-bg); font-weight: 600; color: var(--fg-mute); font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }
section.body tbody tr:hover { background: #fafbfc; }
section.body strong { font-weight: 600; }

nav.toc {
  background: var(--bg-card); border: 1px solid var(--border); border-radius: 8px;
  padding: 18px 24px; margin-bottom: 22px; box-shadow: var(--shadow);
  max-height: 480px; overflow-y: auto;
}
nav.toc h2 { font-size: 14px; text-transform: uppercase; letter-spacing: .04em; color: var(--fg-mute); margin: 0 0 10px; font-weight: 600; }
nav.toc ul { list-style: none; padding-left: 0; margin: 0 0 6px; font-size: 13px; }
nav.toc ul ul { padding-left: 16px; margin: 2px 0 4px; }
nav.toc li { margin: 2px 0; }
nav.toc a { color: var(--accent); text-decoration: none; }
nav.toc a:hover { text-decoration: underline; }
nav.toc details { margin: 4px 0; }
nav.toc summary { font-weight: 600; cursor: pointer; color: var(--fg); font-size: 13px; }
nav.toc summary:hover { color: var(--accent); }

footer.page-footer { color: var(--fg-mute); font-size: 12px; text-align: center; margin-top: 22px; }
footer.page-footer a { color: var(--accent); text-decoration: none; }
""".strip()


# ---------------------------------------------------------------------------
# Inline rendering
# ---------------------------------------------------------------------------


_INLINE_CODE = re.compile(r"`([^`\n]+)`")
_LINK = re.compile(r"\[([^\]]+)\]\(([^)\s]+)(?:\s+\"([^\"]*)\")?\)")
_BOLD = re.compile(r"\*\*([^*\n]+)\*\*")
_ITAL_ASTERISK = re.compile(r"(?<![*\w])\*([^*\n]+)\*(?!\*)")
_ITAL_UNDER = re.compile(r"(?<![_\w])_([^_\n]+)_(?!_)")


def _render_inline(text: str) -> str:
    """Render inline markdown. Inline code spans are masked first so their
    contents don't get further markdown-processed; URLs in link hrefs are
    masked too."""
    placeholders: list[str] = []

    def stash(rendered: str) -> str:
        placeholders.append(rendered)
        return f"\x00P{len(placeholders) - 1}\x00"

    # 1. inline code first: mask its content (already html-escaped) so the
    #    asterisks/underscores inside code stay literal.
    def code_repl(m: re.Match[str]) -> str:
        body = html.escape(m.group(1), quote=False)
        return stash(f"<code>{body}</code>")

    text = _INLINE_CODE.sub(code_repl, text)

    # 2. Links: mask the rendered anchor. The label may already contain
    #    placeholder sentinels from step 1 (inline code), so we don't
    #    recursively re-render — we escape only the non-placeholder portion
    #    and apply bold/italic to that.
    def link_repl(m: re.Match[str]) -> str:
        label = m.group(1)
        url = m.group(2)
        title = m.group(3) or ""
        safe_url = html.escape(url, quote=True)
        # Split the label by placeholder sentinels so we don't escape them.
        parts = re.split(r"(\x00P\d+\x00)", label)
        rebuilt: list[str] = []
        for part in parts:
            if re.fullmatch(r"\x00P\d+\x00", part):
                rebuilt.append(part)
            else:
                safe = html.escape(part, quote=False)
                safe = _BOLD.sub(r"<strong>\1</strong>", safe)
                safe = _ITAL_ASTERISK.sub(r"<em>\1</em>", safe)
                safe = _ITAL_UNDER.sub(r"<em>\1</em>", safe)
                rebuilt.append(safe)
        rendered_label = "".join(rebuilt)
        if title:
            return stash(f'<a href="{safe_url}" title="{html.escape(title, quote=True)}">{rendered_label}</a>')
        return stash(f'<a href="{safe_url}">{rendered_label}</a>')

    text = _LINK.sub(link_repl, text)

    # 3. Now escape the remaining text. Placeholders survive (they contain
    #    \x00 sentinels which aren't HTML-meaningful) because escape leaves
    #    them alone.
    text = html.escape(text, quote=False)

    # 4. Bold then italic. Bold consumes ** pairs first so single * inside
    #    bold doesn't double-trigger italic.
    text = _BOLD.sub(r"<strong>\1</strong>", text)
    text = _ITAL_ASTERISK.sub(r"<em>\1</em>", text)
    text = _ITAL_UNDER.sub(r"<em>\1</em>", text)

    # 5. Restore placeholders.
    def restore(m: re.Match[str]) -> str:
        idx = int(m.group(1))
        return placeholders[idx]

    return re.sub(r"\x00P(\d+)\x00", restore, text)


def _render_inline_text_only(text: str) -> str:
    """Limited inline rendering for link labels: only inline code + escape +
    bold/italic. No nested links."""
    placeholders: list[str] = []

    def stash(rendered: str) -> str:
        placeholders.append(rendered)
        return f"\x00P{len(placeholders) - 1}\x00"

    def code_repl(m: re.Match[str]) -> str:
        body = html.escape(m.group(1), quote=False)
        return stash(f"<code>{body}</code>")

    text = _INLINE_CODE.sub(code_repl, text)
    text = html.escape(text, quote=False)
    text = _BOLD.sub(r"<strong>\1</strong>", text)
    text = _ITAL_ASTERISK.sub(r"<em>\1</em>", text)
    text = _ITAL_UNDER.sub(r"<em>\1</em>", text)

    def restore(m: re.Match[str]) -> str:
        idx = int(m.group(1))
        return placeholders[idx]

    return re.sub(r"\x00P(\d+)\x00", restore, text)


# ---------------------------------------------------------------------------
# Block rendering
# ---------------------------------------------------------------------------


@dataclass
class TocEntry:
    level: int  # 2 or 3
    text: str
    anchor: str


@dataclass
class _State:
    out: list[str] = field(default_factory=list)
    toc: list[TocEntry] = field(default_factory=list)
    used_anchors: set[str] = field(default_factory=set)


_HEADER_RE = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
_FENCE_RE = re.compile(r"^(```|~~~)(.*)$")
_HR_RE = re.compile(r"^(?:---+|\*\*\*+|___+)\s*$")
_BULLET_RE = re.compile(r"^(\s*)([-*+])\s+(.*)$")
_OLIST_RE = re.compile(r"^(\s*)(\d+)\.\s+(.*)$")
_TABLE_SEP_RE = re.compile(r"^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)+\|?\s*$")


def _slugify(text: str, used: set[str]) -> str:
    base = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    if not base:
        base = "section"
    slug = base
    n = 1
    while slug in used:
        n += 1
        slug = f"{base}-{n}"
    used.add(slug)
    return slug


def _is_table_start(lines: list[str], i: int) -> bool:
    if i + 1 >= len(lines):
        return False
    line = lines[i]
    sep = lines[i + 1]
    return "|" in line and _TABLE_SEP_RE.match(sep) is not None


def _split_table_row(line: str) -> list[str]:
    s = line.strip()
    if s.startswith("|"):
        s = s[1:]
    if s.endswith("|"):
        s = s[:-1]
    return [c.strip() for c in s.split("|")]


def _render_table(lines: list[str], i: int, state: _State) -> int:
    header = _split_table_row(lines[i])
    # skip separator
    j = i + 2
    rows: list[list[str]] = []
    while j < len(lines):
        line = lines[j]
        if not line.strip() or "|" not in line:
            break
        rows.append(_split_table_row(line))
        j += 1
    state.out.append("<table>")
    state.out.append("<thead><tr>")
    for cell in header:
        state.out.append(f"<th>{_render_inline(cell)}</th>")
    state.out.append("</tr></thead>")
    state.out.append("<tbody>")
    for row in rows:
        state.out.append("<tr>")
        for cell in row:
            state.out.append(f"<td>{_render_inline(cell)}</td>")
        state.out.append("</tr>")
    state.out.append("</tbody></table>")
    return j


def _render_code_fence(lines: list[str], i: int, state: _State, fence_chars: str, lang: str) -> int:
    body: list[str] = []
    j = i + 1
    while j < len(lines):
        line = lines[j]
        if line.strip().startswith(fence_chars) and line.strip().rstrip("`~") == "":
            j += 1
            break
        body.append(line)
        j += 1
    escaped = html.escape("\n".join(body), quote=False)
    lang_attr = f' class="lang-{html.escape(lang.strip(), quote=True)}"' if lang.strip() else ""
    state.out.append(f"<pre><code{lang_attr}>{escaped}</code></pre>")
    return j


def _render_list(lines: list[str], i: int, state: _State, ordered: bool) -> int:
    """Render a list. Items can span multiple lines; nested lists detected by
    increasing indent on a subsequent bullet line."""
    pat = _OLIST_RE if ordered else _BULLET_RE
    first = pat.match(lines[i])
    assert first is not None
    base_indent = len(first.group(1))
    tag = "ol" if ordered else "ul"
    state.out.append(f"<{tag}>")
    j = i
    while j < len(lines):
        line = lines[j]
        m = pat.match(line)
        # Try the other ordering style too — they don't nest across types here.
        if m is None:
            # Maybe a different list type at the same indent → exit
            other = (_BULLET_RE if ordered else _OLIST_RE).match(line)
            if other and len(other.group(1)) == base_indent:
                break
            # Continuation paragraph: indent > base_indent and non-empty
            if line.strip() == "":
                # Could be end of list or just blank between items
                if j + 1 < len(lines):
                    nxt = lines[j + 1]
                    nxt_m = pat.match(nxt) or (_BULLET_RE if ordered else _OLIST_RE).match(nxt)
                    if nxt_m and len(nxt_m.group(1)) == base_indent:
                        j += 1
                        continue
                break
            # Indented continuation of current item (no marker) → ignore for
            # now; markdown laxness — fold into previous <li>. We track this
            # by stripping the indent and re-rendering inline.
            if state.out and state.out[-1].endswith("</li>"):
                # Append text to the previous li body
                prev = state.out.pop()
                # prev is "<li>...content</li>"; insert before </li>
                inner = prev[len("<li>"):-len("</li>")]
                addition = _render_inline(line.strip())
                state.out.append(f"<li>{inner} {addition}</li>")
                j += 1
                continue
            break
        indent = len(m.group(1))
        if indent < base_indent:
            break
        if indent > base_indent:
            # Nested list: recurse
            ordered_child = _OLIST_RE.match(line) is not None
            j = _render_list(lines, j, state, ordered_child)
            # Attach the nested list to the previous <li> if possible
            if len(state.out) >= 2 and state.out[-1].endswith(f"</{('ol' if ordered_child else 'ul')}>"):
                nested_html = state.out.pop()
                # Last <li> was already closed; we need to inject before its
                # </li>. Find the prior <li>…</li> token.
                # Walk back to find the matching <li>…</li>.
                for k in range(len(state.out) - 1, -1, -1):
                    if state.out[k].endswith("</li>"):
                        state.out[k] = state.out[k][:-len("</li>")] + nested_html + "</li>"
                        break
                else:
                    state.out.append(nested_html)
            continue
        # Same-level item
        content = m.group(3) if not ordered else m.group(3)
        state.out.append(f"<li>{_render_inline(content)}</li>")
        j += 1
    state.out.append(f"</{tag}>")
    return j


def _flush_paragraph(buf: list[str], state: _State) -> None:
    if not buf:
        return
    text = " ".join(line.strip() for line in buf)
    state.out.append(f"<p>{_render_inline(text)}</p>")
    buf.clear()


def _render_blockquote(lines: list[str], i: int, state: _State) -> int:
    body: list[str] = []
    j = i
    while j < len(lines):
        line = lines[j]
        if not line.startswith(">"):
            break
        body.append(line.lstrip(">").lstrip())
        j += 1
    rendered = " ".join(body)
    state.out.append(f"<blockquote><p>{_render_inline(rendered)}</p></blockquote>")
    return j


def _convert_body(markdown_text: str, state: _State) -> None:
    lines = markdown_text.split("\n")
    para: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()

        # Fence
        fence_m = _FENCE_RE.match(line.lstrip())
        if fence_m:
            _flush_paragraph(para, state)
            i = _render_code_fence(lines, i, state, fence_m.group(1), fence_m.group(2))
            continue

        # Blank line
        if not stripped:
            _flush_paragraph(para, state)
            i += 1
            continue

        # HR
        if _HR_RE.match(stripped):
            _flush_paragraph(para, state)
            state.out.append("<hr>")
            i += 1
            continue

        # Header
        hm = _HEADER_RE.match(stripped)
        if hm:
            _flush_paragraph(para, state)
            level = len(hm.group(1))
            content = hm.group(2)
            anchor = _slugify(re.sub(r"<[^>]+>", "", content), state.used_anchors)
            rendered = _render_inline(content)
            state.out.append(f'<h{level} id="{anchor}">{rendered}</h{level}>')
            if level in (2, 3):
                state.toc.append(TocEntry(level=level, text=re.sub(r"<[^>]+>", "", rendered), anchor=anchor))
            i += 1
            continue

        # Table
        if _is_table_start(lines, i):
            _flush_paragraph(para, state)
            i = _render_table(lines, i, state)
            continue

        # Lists
        if _BULLET_RE.match(line):
            _flush_paragraph(para, state)
            i = _render_list(lines, i, state, ordered=False)
            continue
        if _OLIST_RE.match(line):
            _flush_paragraph(para, state)
            i = _render_list(lines, i, state, ordered=True)
            continue

        # Blockquote
        if stripped.startswith(">"):
            _flush_paragraph(para, state)
            i = _render_blockquote(lines, i, state)
            continue

        # Default: paragraph line
        para.append(line)
        i += 1

    _flush_paragraph(para, state)


def render(
    markdown_text: str,
    *,
    title: str,
    subtitle_html: str,
    back_href: str,
    source_path_repo: str,
    include_toc: bool = False,
    generated_at: str | None = None,
) -> str:
    """Render a markdown document into a self-contained HTML page."""
    state = _State()
    _convert_body(markdown_text, state)
    body_html = "\n".join(state.out)

    if generated_at is None:
        generated_at = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")

    toc_html = ""
    if include_toc and state.toc:
        toc_html = _build_toc(state.toc)

    page_title = html.escape(title, quote=False)
    source_safe = html.escape(source_path_repo, quote=False)
    back_safe = html.escape(back_href, quote=True)

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{page_title}</title>
<style>
{CSS}
</style>
</head>
<body>
<div class="container">

<header class="page-header">
  <h1>{page_title}</h1>
  <div class="subtitle">{subtitle_html}</div>
  <div class="subtitle" style="margin-top:6px;">Source: <code>{source_safe}</code></div>
  <div class="back"><a href="{back_safe}">&larr; Back to index</a></div>
</header>

{toc_html}

<section class="body">
{body_html}
</section>

<footer class="page-footer">
  Generated {generated_at} &middot; <a href="{back_safe}">Back to index</a>
</footer>

</div>
</body>
</html>
"""


def _build_toc(entries: list[TocEntry]) -> str:
    """Build a TOC. Groups h3 entries under their preceding h2 inside a
    `<details>` so very large documents (task9-review) stay scannable."""
    lines: list[str] = ['<nav class="toc">', "<h2>Contents</h2>", "<ul>"]
    i = 0
    # If there are very many entries, default-collapse the h3 children.
    many = sum(1 for e in entries if e.level == 3) > 30
    while i < len(entries):
        e = entries[i]
        if e.level == 2:
            # Gather following h3 children
            j = i + 1
            children: list[TocEntry] = []
            while j < len(entries) and entries[j].level == 3:
                children.append(entries[j])
                j += 1
            if children:
                if many:
                    lines.append("<li><details>")
                    lines.append(f'<summary><a href="#{e.anchor}">{e.text}</a> <span style="color:var(--fg-mute);font-weight:400;">({len(children)})</span></summary>')
                    lines.append("<ul>")
                    for c in children:
                        lines.append(f'<li><a href="#{c.anchor}">{c.text}</a></li>')
                    lines.append("</ul></details></li>")
                else:
                    lines.append(f'<li><a href="#{e.anchor}">{e.text}</a><ul>')
                    for c in children:
                        lines.append(f'<li><a href="#{c.anchor}">{c.text}</a></li>')
                    lines.append("</ul></li>")
            else:
                lines.append(f'<li><a href="#{e.anchor}">{e.text}</a></li>')
            i = j
        else:
            # Orphan h3 (no preceding h2 captured): render at top level
            lines.append(f'<li><a href="#{e.anchor}">{e.text}</a></li>')
            i += 1
    lines.append("</ul></nav>")
    return "\n".join(lines)


def render_plain_text(
    text: str,
    *,
    title: str,
    subtitle_html: str,
    back_href: str,
    source_path_repo: str,
    generated_at: str | None = None,
) -> str:
    """Render a plain-text file into the same HTML shell as `render()`,
    wrapping the content in a single `<pre>`."""
    if generated_at is None:
        generated_at = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    escaped = html.escape(text, quote=False)
    page_title = html.escape(title, quote=False)
    source_safe = html.escape(source_path_repo, quote=False)
    back_safe = html.escape(back_href, quote=True)
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{page_title}</title>
<style>
{CSS}
</style>
</head>
<body>
<div class="container">

<header class="page-header">
  <h1>{page_title}</h1>
  <div class="subtitle">{subtitle_html}</div>
  <div class="subtitle" style="margin-top:6px;">Source: <code>{source_safe}</code></div>
  <div class="back"><a href="{back_safe}">&larr; Back to index</a></div>
</header>

<section class="body">
<pre>{escaped}</pre>
</section>

<footer class="page-footer">
  Generated {generated_at} &middot; <a href="{back_safe}">Back to index</a>
</footer>

</div>
</body>
</html>
"""
