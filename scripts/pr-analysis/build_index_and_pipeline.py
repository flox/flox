#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Build the master index page, pipeline-architecture page, and HTML
renderings of every markdown analysis artifact.

The script is pure stdlib. It:

1. Queries the SQLite DB for KPIs.
2. Renders every entry in `MD_MANIFEST` to its sibling `.html` via the
   `lib.md2html` helper. "Back to index" links are computed with
   `os.path.relpath` so files at any depth resolve to the index correctly.
3. Renders the master index page, with cards for every existing artifact.
4. Renders the pipeline-architecture page.

The journey log is intentionally NOT regenerated from its markdown via
md2html: the existing journey HTML has hand-built timeline / iteration card
visualizations that a generic converter would destroy. That file is
hand-edited separately. The journey log card in the index still points at
the existing journey HTML.
"""
from __future__ import annotations

import datetime as dt
import os
import sqlite3
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

WORKTREE = Path(__file__).resolve().parents[2]
DB_PATH = WORKTREE / "scripts" / "pr-analysis" / "data" / "pr_analysis.db"

INDEX_PATH = WORKTREE / "rust-pr-analysis-index-01.html"
PIPELINE_PATH = WORKTREE / "rust-pr-analysis-pipeline-01.html"

# Allow `from lib import md2html` when run via `uv run --script`.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from lib import md2html  # noqa: E402


# ---------------------------------------------------------------------------
# Manifest of markdown / text → HTML conversions
# ---------------------------------------------------------------------------


@dataclass
class MdEntry:
    src: Path
    dst: Path
    title: str
    subtitle: str
    include_toc: bool = False
    plain_text: bool = False


def md_manifest() -> list[MdEntry]:
    rel = lambda *p: WORKTREE.joinpath(*p)  # noqa: E731
    return [
        # Worktree-root artifacts
        MdEntry(
            src=rel("rust-pr-analysis-summary-prompt-01.md"),
            dst=rel("rust-pr-analysis-summary-prompt-01.html"),
            title="Rust PR Analysis — Summary Prompt",
            subtitle="Self-contained brief for resuming the work in a fresh session.",
        ),
        # Original plan
        MdEntry(
            src=rel("docs/superpowers/plans/2026-05-16-flox-rust-pr-analysis-skill.md"),
            dst=rel("docs/superpowers/plans/2026-05-16-flox-rust-pr-analysis-skill.html"),
            title="Original Implementation Plan (2026-05-16)",
            subtitle="13-task plan written before any code was committed.",
            include_toc=True,
        ),
        # findings/ markdown deliverables
        MdEntry(
            src=rel("scripts/pr-analysis/findings/task9-review.md"),
            dst=rel("scripts/pr-analysis/findings/task9-review.html"),
            title="Task 9 — Rule-by-rule Review",
            subtitle="Primary substantive analysis deliverable: every finding with source comment, diff hunk, merged code, reviewer voices, AGENTS.md status.",
            include_toc=True,
        ),
        MdEntry(
            src=rel("scripts/pr-analysis/findings/task8-full-corpus.md"),
            dst=rel("scripts/pr-analysis/findings/task8-full-corpus.html"),
            title="Task 8 — Full Corpus Digest",
            subtitle="8-month window (2025-09-17 → 2026-05-17) digest covering 216 Rust-touching PRs from 335 merged.",
            include_toc=True,
        ),
        MdEntry(
            src=rel("scripts/pr-analysis/findings/iter4-comparison.md"),
            dst=rel("scripts/pr-analysis/findings/iter4-comparison.html"),
            title="Iteration 4 — Second-Window Validation",
            subtitle="2025-09-16 → 2025-11-15 pilot comparison to verify calibration generalises beyond the original window.",
        ),
        MdEntry(
            src=rel("scripts/pr-analysis/findings/pilot-retro.md"),
            dst=rel("scripts/pr-analysis/findings/pilot-retro.html"),
            title="Pilot Retrospective (iter 1–3)",
            subtitle="Pre-Task-8 retrospective digest. Now superseded by the journey log + Task 8 results, kept for traceability.",
        ),
        MdEntry(
            src=rel("scripts/pr-analysis/findings/gap-report.md"),
            dst=rel("scripts/pr-analysis/findings/gap-report.html"),
            title="AGENTS.md Gap Report",
            subtitle="30 proposed amendments + 2 new-section proposals derived from 944 review comments across 216 PRs.",
            include_toc=True,
        ),
        MdEntry(
            src=rel("scripts/pr-analysis/findings/other-cluster-candidates.txt"),
            dst=rel("scripts/pr-analysis/findings/other-cluster-candidates.html"),
            title="Task 8.5 — 'other'-bucket Cluster Candidates",
            subtitle="High-confidence taxonomy='other' rule statements clustered as candidates for taxonomy expansion.",
            plain_text=True,
        ),
        # cli/ tree CLAUDE.md files
        MdEntry(
            src=rel("cli/CLAUDE.md"),
            dst=rel("cli/CLAUDE.html"),
            title="cli/CLAUDE.md — Rust cross-cutting rules",
            subtitle="Per-tree directive pointing at the two Rust review skills and the three per-area CLAUDE.md files.",
        ),
        MdEntry(
            src=rel("cli/flox/src/commands/CLAUDE.md"),
            dst=rel("cli/flox/src/commands/CLAUDE.html"),
            title="commands/CLAUDE.md — Hot-area rules",
            subtitle="Command-implementation-specific rules synthesized from PR reviews.",
        ),
        MdEntry(
            src=rel("cli/flox-rust-sdk/src/models/environment/CLAUDE.md"),
            dst=rel("cli/flox-rust-sdk/src/models/environment/CLAUDE.html"),
            title="models/environment/CLAUDE.md — Hot-area rules",
            subtitle="Environment-model-specific rules synthesized from PR reviews.",
        ),
        MdEntry(
            src=rel("cli/flox-rust-sdk/src/providers/CLAUDE.md"),
            dst=rel("cli/flox-rust-sdk/src/providers/CLAUDE.html"),
            title="providers/CLAUDE.md — Hot-area rules",
            subtitle="Provider-trait-specific rules synthesized from PR reviews.",
        ),
        # Skills
        MdEntry(
            src=rel(".claude/skills/flox-rust-review/SKILL.md"),
            dst=rel(".claude/skills/flox-rust-review/SKILL.html"),
            title="flox-rust-review — Skill",
            subtitle="Cross-cutting Rust review skill. Substantive rules (correctness, error handling, type safety, panic discipline) mined from 944 comments / 216 PRs.",
        ),
        MdEntry(
            src=rel(".claude/skills/flox-rust-stylistic-conventions/SKILL.md"),
            dst=rel(".claude/skills/flox-rust-stylistic-conventions/SKILL.html"),
            title="flox-rust-stylistic-conventions — Skill",
            subtitle="Stylistic Rust conventions skill. 54 gap-candidate stylistic rules extracted from the noise-filter deep-dive.",
        ),
    ]


# ---------------------------------------------------------------------------
# CSS palette for index/pipeline (unchanged from prior version)
# ---------------------------------------------------------------------------


CSS_PALETTE = """
:root {
  --fg: #1c1f24; --fg-mute: #5a6270; --bg: #fbfbfa; --bg-card: #ffffff;
  --border: #e4e6eb; --accent: #3b6bd6; --good: #2f8a52; --warn: #c98a17;
  --bad: #c44545; --neutral: #6c757d; --code-bg: #f3f4f6;
  --shadow: 0 1px 2px rgba(0,0,0,.04), 0 4px 12px rgba(0,0,0,.04);
}
* { box-sizing: border-box; }
body {
  font: 15px/1.55 -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
  color: var(--fg); background: var(--bg); margin: 0; padding: 32px 24px 80px;
}
.container { max-width: 1080px; margin: 0 auto; }
header.page-header {
  background: var(--bg-card); border: 1px solid var(--border);
  border-radius: 8px; padding: 24px 28px; box-shadow: var(--shadow);
  margin-bottom: 28px;
}
header.page-header h1 { margin: 0 0 4px; font-size: 26px; font-weight: 600; letter-spacing: -.01em; }
header.page-header .subtitle { color: var(--fg-mute); font-size: 14px; margin-bottom: 18px; }
header.page-header p.blurb { color: var(--fg); font-size: 14px; line-height: 1.6; margin: 0 0 16px; }
.stat-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 14px; margin-top: 6px; }
.stat { background: var(--code-bg); border-radius: 6px; padding: 10px 12px; }
.stat .label { font-size: 11px; text-transform: uppercase; letter-spacing: .04em; color: var(--fg-mute); }
.stat .value { font-size: 17px; font-weight: 600; font-variant-numeric: tabular-nums; margin-top: 2px; }
section {
  background: var(--bg-card); border: 1px solid var(--border);
  border-radius: 8px; padding: 24px 28px; margin-bottom: 22px;
  box-shadow: var(--shadow);
}
section h2 { margin: 0 0 14px; font-size: 19px; font-weight: 600; letter-spacing: -.005em; }
section h3 { margin: 18px 0 8px; font-size: 13px; font-weight: 600; color: var(--fg-mute); text-transform: uppercase; letter-spacing: .04em; }
section h4 { margin: 14px 0 6px; font-size: 14px; font-weight: 600; }
p { margin: 0 0 10px; }
code { font-family: "SF Mono", Menlo, Consolas, monospace; font-size: 13px; background: var(--code-bg); padding: 1px 5px; border-radius: 3px; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
.pill { display: inline-block; padding: 2px 8px; border-radius: 999px; font-size: 11px; font-weight: 600; letter-spacing: .02em; }
.pill.good { background: #e3f3e8; color: #1e5a35; }
.pill.warn { background: #faecd1; color: #7c5510; }
.pill.bad  { background: #f8dada; color: #7a2929; }
.pill.neutral { background: #e8e9ec; color: #495057; }
.pill.accent { background: #e9eef9; color: #1e3a7a; }
.chart-wrap { overflow-x: auto; }
.footer { color: var(--fg-mute); font-size: 12px; margin-top: 12px; }
table.std { border-collapse: collapse; font-size: 13px; width: 100%; }
table.std th, table.std td { border: 1px solid var(--border); padding: 8px 10px; text-align: left; vertical-align: top; }
table.std th { background: var(--code-bg); font-weight: 600; color: var(--fg-mute); text-transform: uppercase; font-size: 11px; letter-spacing: .04em; }
table.std td code { font-size: 12px; }
ul.tight, ol.tight { margin: 6px 0 10px; padding-left: 20px; }
ul.tight li, ol.tight li { margin: 2px 0; }
.cards { display: grid; grid-template-columns: 1fr; gap: 14px; }
.card {
  border: 1px solid var(--border); border-radius: 8px; padding: 16px 18px; background: var(--bg-card);
}
.card .ttl { display: flex; align-items: center; gap: 10px; margin-bottom: 4px; }
.card .ttl h3 { margin: 0; font-size: 16px; font-weight: 600; color: var(--fg); text-transform: none; letter-spacing: 0; }
.card .blurb { font-size: 14px; color: var(--fg); margin: 6px 0 10px; }
.card .meta { font-size: 12px; color: var(--fg-mute); margin-bottom: 8px; }
.card .meta b { color: var(--fg); font-weight: 600; }
.card .open { font-size: 13px; font-weight: 600; }
.card .source { font-size: 12px; color: var(--fg-mute); margin-top: 4px; }
.card .source a { color: var(--fg-mute); text-decoration: underline; }
.kbd { font-family: "SF Mono", Menlo, Consolas, monospace; font-size: 12px; background: #f0f0ef; border: 1px solid var(--border); padding: 1px 5px; border-radius: 3px; color: #333; }
.badge-row { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 6px; }
""".strip()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def db_stats() -> dict[str, int | str]:
    conn = sqlite3.connect(DB_PATH)
    cur = conn.cursor()
    out: dict[str, int | str] = {}
    out["pr_count"] = cur.execute("SELECT COUNT(*) FROM pr").fetchone()[0]
    out["line_comment_count"] = cur.execute("SELECT COUNT(*) FROM line_comment").fetchone()[0]
    out["classified_count"] = cur.execute("SELECT COUNT(*) FROM classification").fetchone()[0]
    out["noise_count"] = cur.execute("SELECT COUNT(*) FROM line_comment WHERE is_noise=1").fetchone()[0]
    out["finding_count"] = cur.execute("SELECT COUNT(*) FROM finding").fetchone()[0]
    out["cross_cutting_count"] = cur.execute(
        "SELECT COUNT(*) FROM finding WHERE scope='cross-cutting'"
    ).fetchone()[0]
    out["review_summary_count"] = cur.execute("SELECT COUNT(*) FROM review_summary").fetchone()[0]
    out["pr_comment_count"] = cur.execute("SELECT COUNT(*) FROM pr_comment").fetchone()[0]
    out["reviewer_count"] = cur.execute("SELECT COUNT(*) FROM reviewer").fetchone()[0]
    row = cur.execute("SELECT MIN(merged_at), MAX(merged_at) FROM pr").fetchone()
    out["window_start"] = (row[0] or "")[:10]
    out["window_end"] = (row[1] or "")[:10]
    conn.close()
    return out


def git_short_sha() -> str:
    res = subprocess.run(
        ["git", "log", "-1", "--format=%h"],
        cwd=WORKTREE,
        capture_output=True,
        text=True,
        check=False,
    )
    return res.stdout.strip() or "unknown"


def commits_in_build() -> int:
    res = subprocess.run(
        ["git", "log", "--oneline", "--grep=pr-analysis"],
        cwd=WORKTREE,
        capture_output=True,
        text=True,
        check=False,
    )
    return len([ln for ln in res.stdout.splitlines() if ln.strip()])


def file_kb(path: Path) -> str:
    if not path.exists():
        return "—"
    sz = path.stat().st_size
    if sz < 1024:
        return f"{sz} B"
    return f"{sz // 1024} KB"


def file_lines(path: Path) -> int:
    if not path.exists():
        return 0
    with open(path, "rb") as f:
        return sum(1 for _ in f)


def today_iso() -> str:
    return dt.date.today().isoformat()


def relpath(src: Path, base: Path) -> str:
    return os.path.relpath(src, base).replace(os.sep, "/")


# ---------------------------------------------------------------------------
# MD → HTML conversion driver
# ---------------------------------------------------------------------------


def convert_md_entries(entries: list[MdEntry]) -> list[tuple[Path, int]]:
    """Convert all manifest entries; return list of (path, bytes_written)."""
    results: list[tuple[Path, int]] = []
    for e in entries:
        if not e.src.exists():
            print(f"SKIP (missing source): {e.src.relative_to(WORKTREE)}", file=sys.stderr)
            continue
        text = e.src.read_text(encoding="utf-8")
        back = relpath(INDEX_PATH, e.dst.parent)
        source_repo = str(e.src.relative_to(WORKTREE))
        subtitle_html = e.subtitle  # already plain HTML, no escaping (we author it)
        if e.plain_text:
            html_doc = md2html.render_plain_text(
                text,
                title=e.title,
                subtitle_html=subtitle_html,
                back_href=back,
                source_path_repo=source_repo,
            )
        else:
            html_doc = md2html.render(
                text,
                title=e.title,
                subtitle_html=subtitle_html,
                back_href=back,
                source_path_repo=source_repo,
                include_toc=e.include_toc,
            )
        e.dst.parent.mkdir(parents=True, exist_ok=True)
        e.dst.write_text(html_doc, encoding="utf-8")
        results.append((e.dst, len(html_doc.encode("utf-8"))))
    return results


# ---------------------------------------------------------------------------
# Index rendering
# ---------------------------------------------------------------------------


def render_card(
    title: str,
    primary_href: str,
    badge: str,
    badge_class: str,
    blurb_html: str,
    stats: list[tuple[str, str]],
    md_source_href: str | None = None,
) -> str:
    stat_pairs = "".join(
        f'<div class="stat"><div class="label">{k}</div><div class="value">{v}</div></div>'
        for k, v in stats
    )
    source_line = (
        f'<div class="source">Markdown source: <a href="{md_source_href}">{md_source_href}</a></div>'
        if md_source_href
        else ""
    )
    return f"""
<div class="card">
  <div class="badge-row">
    <span class="pill {badge_class}">{badge}</span>
  </div>
  <div class="ttl"><h3><a href="{primary_href}">{title}</a></h3></div>
  <div class="blurb">{blurb_html}</div>
  <div class="stat-grid">{stat_pairs}</div>
  <div class="open"><a href="{primary_href}">Open &rarr;</a></div>
  {source_line}
</div>
""".strip()


def render_index(stats: dict[str, int | str], sha: str, commit_count: int) -> str:
    css = CSS_PALETTE
    blurb = (
        "This project mines 6-8 months of merged Rust PRs in <code>flox/flox</code> "
        "to extract review-validated coding rules. The analysis window is "
        f"<b>{stats['window_start']}</b> &rarr; <b>{stats['window_end']}</b>, covering "
        f"<b>{stats['pr_count']} PRs</b>, <b>{stats['classified_count']:,} classified "
        f"comments</b>, and <b>{stats['finding_count']} findings</b>. The pipeline runs "
        "without a paid API key by orchestrating Haiku subagents in batches from inside "
        "the Claude Code session itself. Synthesis (Sonnet 4.6) produced two reusable "
        "Claude Code skills and four per-tree <code>CLAUDE.md</code> files."
    )

    # --- Artifacts (existing top-level reports) ---
    artifact_cards = [
        render_card(
            title="Summary Prompt",
            primary_href="rust-pr-analysis-summary-prompt-01.html",
            md_source_href="rust-pr-analysis-summary-prompt-01.md",
            badge="Prompt",
            badge_class="accent",
            blurb_html=(
                "Self-contained brief describing what's already built and the "
                "outputs still wanted. Paste into a fresh Claude session to resume "
                "the work without losing context."
            ),
            stats=[
                ("Lines", f"{file_lines(WORKTREE / 'rust-pr-analysis-summary-prompt-01.md'):,}"),
                ("Size", file_kb(WORKTREE / "rust-pr-analysis-summary-prompt-01.html")),
                ("Audience", "Resuming session"),
            ],
        ),
        render_card(
            title="Journey Log",
            primary_href="rust-pr-analysis-jouney-01.html",
            md_source_href="rust-pr-analysis-jouney-01.md",
            badge="Journey",
            badge_class="neutral",
            blurb_html=(
                "Chronological log of 79 events &mdash; from worktree creation through "
                "the Sonnet 4.6 re-classification pass, the noise-deep-dive, full-corpus "
                "Task 8, and the synthesis dispatch that produced the two skills, three "
                "area CLAUDE.md files, and the gap report. Hand-built timeline + iteration "
                "card visualizations alongside the event list."
            ),
            stats=[
                ("Events", "79"),
                ("Phases", "6 (5 done, 1 wrap-up)"),
                ("Size", file_kb(WORKTREE / "rust-pr-analysis-jouney-01.html")),
            ],
        ),
        render_card(
            title="Main Analytics Dashboard",
            primary_href="rust-pr-analysis-dashboard-01.html",
            badge="Dashboard",
            badge_class="good",
            blurb_html=(
                "The &ldquo;what the corpus looks like&rdquo; snapshot: KPIs, PRs over time, "
                "top reviewers / authors / committers, lines-of-code by month by filetype, "
                "reviewer &times; area heatmap, area/taxonomy segmentation, "
                "<code>was_addressed &times; thread_resolved</code> cross-tab, and the "
                "cross-cutting findings."
            ),
            stats=[
                ("PRs", str(stats["pr_count"])),
                ("Findings", str(stats["finding_count"])),
                ("Size", file_kb(WORKTREE / "rust-pr-analysis-dashboard-01.html")),
            ],
        ),
        render_card(
            title="Noise Filter Deep-Dive",
            primary_href="rust-pr-analysis-noise-deep-dive-01.html",
            badge="Deep-dive",
            badge_class="warn",
            blurb_html=(
                "Forensic audit of the 87 comments dropped by the noise filter "
                "(45 suggestion-only, 22 lgtm, 16 url-only, 4 praise/nit), a tier-rate "
                "sanity check, and the 163 stylistic-taxonomy classifications &mdash; 54 of "
                "which are gap candidates not yet in AGENTS.md. Motivates the dedicated "
                "<code>flox-rust-stylistic-conventions</code> skill."
            ),
            stats=[
                ("Noise filtered", str(stats["noise_count"])),
                ("Stylistic gap candidates", "54"),
                ("Size", file_kb(WORKTREE / "rust-pr-analysis-noise-deep-dive-01.html")),
            ],
        ),
        render_card(
            title="Pipeline Architecture &amp; Process",
            primary_href="rust-pr-analysis-pipeline-01.html",
            badge="Process",
            badge_class="accent",
            blurb_html=(
                "How the ETL works: ingest &rarr; classify &rarr; aggregate &rarr; synthesize "
                "&rarr; visualize. Schema ER diagram, configuration knobs, the subagent "
                "orchestration model, known pitfalls, and the four invariants enforced by "
                "<code>audit_coverage.py</code>."
            ),
            stats=[
                ("Stages", "5"),
                ("Tables", "10"),
                ("Size", file_kb(WORKTREE / "rust-pr-analysis-pipeline-01.html")),
            ],
        ),
        render_card(
            title="Original Implementation Plan",
            primary_href="docs/superpowers/plans/2026-05-16-flox-rust-pr-analysis-skill.html",
            md_source_href="docs/superpowers/plans/2026-05-16-flox-rust-pr-analysis-skill.md",
            badge="Plan",
            badge_class="neutral",
            blurb_html=(
                "The 13-task plan written before any code was committed. Useful for "
                "understanding original intent vs what actually shipped, including which "
                "tasks expanded, which were dropped, and which produced unexpected "
                "sub-deliverables."
            ),
            stats=[
                ("Tasks", "13"),
                ("Authored", "2026-05-16"),
                ("Size", file_kb(WORKTREE / "docs/superpowers/plans/2026-05-16-flox-rust-pr-analysis-skill.html")),
            ],
        ),
    ]
    cards_block = "\n".join(artifact_cards)

    # --- Final analysis outputs (findings/ HTML reports — natively HTML, not generated by md2html) ---
    final_cards = [
        render_card(
            title="findings/journey-report.html",
            primary_href="scripts/pr-analysis/findings/journey-report.html",
            badge="Calibration journey",
            badge_class="accent",
            blurb_html=(
                "Calibration-focused journey, complements the chronological worktree-root "
                "journey log. 10 sections walking iter1&rarr;iter2&rarr;iter3&rarr;iter4 of "
                "the heuristic-calibration story (clustering threshold sweep, AGENTS.md "
                "matching swings, MiniLM embedding adoption, Haiku self-doubt fix)."
            ),
            stats=[
                ("Sections", "10"),
                ("Size", file_kb(WORKTREE / "scripts/pr-analysis/findings/journey-report.html")),
                ("Role", "Calibration story"),
            ],
        ),
        render_card(
            title="findings/results-summary.html",
            primary_href="scripts/pr-analysis/findings/results-summary.html",
            badge="Final results",
            badge_class="good",
            blurb_html=(
                "AGENTS.md before/after diff with greenlight inserts &mdash; the integration "
                "view showing exactly what would change if the gap-report amendments were "
                "applied. 5 sections."
            ),
            stats=[
                ("Sections", "5"),
                ("Size", file_kb(WORKTREE / "scripts/pr-analysis/findings/results-summary.html")),
                ("Role", "Integration view"),
            ],
        ),
        render_card(
            title="findings/gap-report.html",
            primary_href="scripts/pr-analysis/findings/gap-report.html",
            md_source_href="scripts/pr-analysis/findings/gap-report.md",
            badge="Gap report",
            badge_class="warn",
            blurb_html=(
                "30 proposed AGENTS.md amendments grouped thematically, plus 2 new-section "
                "proposals. Each amendment cites the originating PR(s), confidence tier "
                "(T1-accepted / T1-raised / T2-only), and proposed AGENTS.md text."
            ),
            stats=[
                ("Amendments", "30 + 2 sections"),
                ("Lines", f"{file_lines(WORKTREE / 'scripts/pr-analysis/findings/gap-report.md'):,}"),
                ("Size", file_kb(WORKTREE / "scripts/pr-analysis/findings/gap-report.html")),
            ],
        ),
    ]
    final_cards_block = "\n".join(final_cards)

    # --- Synthesized outputs (skills + per-area CLAUDE.md) ---
    synth_cards = [
        render_card(
            title=".claude/skills/flox-rust-review/SKILL.html",
            primary_href=".claude/skills/flox-rust-review/SKILL.html",
            md_source_href=".claude/skills/flox-rust-review/SKILL.md",
            badge="Skill",
            badge_class="good",
            blurb_html=(
                "<b>Cross-cutting Rust review skill.</b> Substantive rules covering error "
                "handling, type safety at boundaries, semantic correctness, testing "
                "patterns, provider-trait design, manifest-usage discipline, panic "
                "discipline. Mined from 944 review comments across 216 PRs."
            ),
            stats=[
                ("Lines", f"{file_lines(WORKTREE / '.claude/skills/flox-rust-review/SKILL.md'):,}"),
                ("Size", file_kb(WORKTREE / ".claude/skills/flox-rust-review/SKILL.html")),
                ("Role", "Cross-cutting"),
            ],
        ),
        render_card(
            title=".claude/skills/flox-rust-stylistic-conventions/SKILL.html",
            primary_href=".claude/skills/flox-rust-stylistic-conventions/SKILL.html",
            md_source_href=".claude/skills/flox-rust-stylistic-conventions/SKILL.md",
            badge="Skill",
            badge_class="good",
            blurb_html=(
                "<b>Stylistic Rust conventions skill.</b> 54 gap-candidate stylistic rules "
                "extracted from the noise-filter deep-dive &mdash; rules reviewers enforce "
                "but that aren't yet captured in AGENTS.md."
            ),
            stats=[
                ("Lines", f"{file_lines(WORKTREE / '.claude/skills/flox-rust-stylistic-conventions/SKILL.md'):,}"),
                ("Size", file_kb(WORKTREE / ".claude/skills/flox-rust-stylistic-conventions/SKILL.html")),
                ("Role", "Stylistic"),
            ],
        ),
        render_card(
            title="cli/CLAUDE.html",
            primary_href="cli/CLAUDE.html",
            md_source_href="cli/CLAUDE.md",
            badge="Per-tree",
            badge_class="accent",
            blurb_html=(
                "Tree-level rule directive at <code>cli/</code>. Points at the two skills "
                "above and the three per-area CLAUDE.md files. Loaded by Claude Code when "
                "editing any file under <code>cli/</code>."
            ),
            stats=[
                ("Lines", f"{file_lines(WORKTREE / 'cli/CLAUDE.md'):,}"),
                ("Size", file_kb(WORKTREE / "cli/CLAUDE.html")),
                ("Role", "Per-tree directive"),
            ],
        ),
        render_card(
            title="cli/flox/src/commands/CLAUDE.html",
            primary_href="cli/flox/src/commands/CLAUDE.html",
            md_source_href="cli/flox/src/commands/CLAUDE.md",
            badge="Per-area",
            badge_class="accent",
            blurb_html=(
                "Command-implementation-specific rules synthesized from PR reviews touching "
                "<code>cli/flox/src/commands/</code>. The largest hot area in the corpus "
                "(207 comments)."
            ),
            stats=[
                ("Lines", f"{file_lines(WORKTREE / 'cli/flox/src/commands/CLAUDE.md'):,}"),
                ("Size", file_kb(WORKTREE / "cli/flox/src/commands/CLAUDE.html")),
                ("Role", "Hot area"),
            ],
        ),
        render_card(
            title="cli/flox-rust-sdk/src/models/environment/CLAUDE.html",
            primary_href="cli/flox-rust-sdk/src/models/environment/CLAUDE.html",
            md_source_href="cli/flox-rust-sdk/src/models/environment/CLAUDE.md",
            badge="Per-area",
            badge_class="accent",
            blurb_html=(
                "Environment-model-specific rules. Synthesized from PR reviews touching "
                "<code>cli/flox-rust-sdk/src/models/environment/</code> (115 comments)."
            ),
            stats=[
                ("Lines", f"{file_lines(WORKTREE / 'cli/flox-rust-sdk/src/models/environment/CLAUDE.md'):,}"),
                ("Size", file_kb(WORKTREE / "cli/flox-rust-sdk/src/models/environment/CLAUDE.html")),
                ("Role", "Hot area"),
            ],
        ),
        render_card(
            title="cli/flox-rust-sdk/src/providers/CLAUDE.html",
            primary_href="cli/flox-rust-sdk/src/providers/CLAUDE.html",
            md_source_href="cli/flox-rust-sdk/src/providers/CLAUDE.md",
            badge="Per-area",
            badge_class="accent",
            blurb_html=(
                "Provider-trait-specific rules. Synthesized from PR reviews touching "
                "<code>cli/flox-rust-sdk/src/providers/</code> (103 comments)."
            ),
            stats=[
                ("Lines", f"{file_lines(WORKTREE / 'cli/flox-rust-sdk/src/providers/CLAUDE.md'):,}"),
                ("Size", file_kb(WORKTREE / "cli/flox-rust-sdk/src/providers/CLAUDE.html")),
                ("Role", "Hot area"),
            ],
        ),
    ]
    synth_cards_block = "\n".join(synth_cards)

    # --- Rule-level analysis findings/ ---
    findings_dir = WORKTREE / "scripts" / "pr-analysis" / "findings"
    findings_cards = [
        render_card(
            title="task9-review.html",
            primary_href="scripts/pr-analysis/findings/task9-review.html",
            md_source_href="scripts/pr-analysis/findings/task9-review.md",
            badge="Markdown",
            badge_class="neutral",
            blurb_html=(
                "Rule-by-rule review document. Every finding rendered with source comment, "
                "diff hunk, merged final code, reviewer voices, and AGENTS.md status. "
                "<b>The primary substantive analysis deliverable.</b> TOC at top groups "
                "the 128 finding anchors under their parent sections."
            ),
            stats=[
                ("Findings rendered", "128"),
                ("Lines (MD)", f"{file_lines(findings_dir / 'task9-review.md'):,}"),
                ("Size (HTML)", file_kb(findings_dir / "task9-review.html")),
            ],
        ),
        render_card(
            title="task8-full-corpus.html",
            primary_href="scripts/pr-analysis/findings/task8-full-corpus.html",
            md_source_href="scripts/pr-analysis/findings/task8-full-corpus.md",
            badge="Markdown",
            badge_class="neutral",
            blurb_html=(
                "Task 8 full-corpus run results document &mdash; 8-month window "
                "(2025-09-17 &rarr; 2026-05-17) digest covering 216 Rust-touching PRs "
                "from 335 merged."
            ),
            stats=[
                ("Lines (MD)", f"{file_lines(findings_dir / 'task8-full-corpus.md'):,}"),
                ("Size (HTML)", file_kb(findings_dir / "task8-full-corpus.html")),
                ("Role", "Corpus digest"),
            ],
        ),
        render_card(
            title="iter4-comparison.html",
            primary_href="scripts/pr-analysis/findings/iter4-comparison.html",
            md_source_href="scripts/pr-analysis/findings/iter4-comparison.md",
            badge="Markdown",
            badge_class="neutral",
            blurb_html=(
                "Iteration-4 pilot comparison &mdash; second-window validation across "
                "2025-09-16 &rarr; 2025-11-15 to verify calibration generalises beyond "
                "the original recent-month window."
            ),
            stats=[
                ("Lines (MD)", f"{file_lines(findings_dir / 'iter4-comparison.md'):,}"),
                ("Size (HTML)", file_kb(findings_dir / "iter4-comparison.html")),
                ("Role", "Validation"),
            ],
        ),
        render_card(
            title="pilot-retro.html",
            primary_href="scripts/pr-analysis/findings/pilot-retro.html",
            md_source_href="scripts/pr-analysis/findings/pilot-retro.md",
            badge="Markdown",
            badge_class="neutral",
            blurb_html=(
                "Pilot retrospective digest from iterations 1&ndash;3 (now superseded by "
                "the journey log + Task 8 results, kept for traceability)."
            ),
            stats=[
                ("Lines (MD)", f"{file_lines(findings_dir / 'pilot-retro.md'):,}"),
                ("Size (HTML)", file_kb(findings_dir / "pilot-retro.html")),
                ("Role", "Retrospective"),
            ],
        ),
        render_card(
            title="other-cluster-candidates.html",
            primary_href="scripts/pr-analysis/findings/other-cluster-candidates.html",
            md_source_href="scripts/pr-analysis/findings/other-cluster-candidates.txt",
            badge="Text",
            badge_class="neutral",
            blurb_html=(
                "Task 8.5 input: high-confidence <code>taxonomy='other'</code> rule "
                "statements clustered as candidates for taxonomy expansion."
            ),
            stats=[
                ("Lines (TXT)", f"{file_lines(findings_dir / 'other-cluster-candidates.txt'):,}"),
                ("Size (HTML)", file_kb(findings_dir / "other-cluster-candidates.html")),
                ("Role", "Taxonomy input"),
            ],
        ),
    ]
    findings_cards_block = "\n".join(findings_cards)

    now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    snapshot_date = today_iso()

    # Count of artifact cards displayed (top-row only) for the header tile.
    artifact_count = (
        len(artifact_cards) + len(final_cards) + len(synth_cards) + len(findings_cards)
    )

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Flox Rust PR Analysis &mdash; Artifact Index</title>
<style>
{css}
</style>
</head>
<body>
<div class="container">

<header class="page-header">
  <h1>Flox Rust PR Analysis &mdash; Artifact Index</h1>
  <div class="subtitle">
    Landing page for everything produced by the
    <code>rust-pr-analysis-skill</code> worktree &middot; merged-PR window
    <b>{stats['window_start']}</b> &rarr; <b>{stats['window_end']}</b>
  </div>
  <p class="blurb">{blurb}</p>
  <div class="stat-grid">
    <div class="stat"><div class="label">Artifacts</div><div class="value">{artifact_count}</div></div>
    <div class="stat"><div class="label">PRs analysed</div><div class="value">{stats['pr_count']}</div></div>
    <div class="stat"><div class="label">Classified comments</div><div class="value">{stats['classified_count']:,}</div></div>
    <div class="stat"><div class="label">Findings</div><div class="value">{stats['finding_count']}</div></div>
    <div class="stat"><div class="label">Commits</div><div class="value">{commit_count}</div></div>
    <div class="stat"><div class="label">Skills + CLAUDE.md</div><div class="value">2 + 4</div></div>
    <div class="stat"><div class="label">Latest commit</div><div class="value"><code>{sha}</code></div></div>
  </div>
  <p style="margin-top:14px;font-size:12px;color:var(--fg-mute);">
    Stats reflect DB state at commit <code>{sha}</code> / timestamp
    <b>{snapshot_date}</b>. Numbers re-query the live DB on every regeneration.
    Every card links to HTML primary; the markdown source is linked underneath
    where it exists.
  </p>
</header>

<section>
  <h2>How to read these</h2>
  <ul class="tight">
    <li><b>First-time reader</b> &rarr; start with the <b>Journey Log</b> (the <i>why</i> and <i>how</i>).</li>
    <li><b>Skim the corpus</b> &rarr; open the <b>Main Dashboard</b> (the <i>what</i>).</li>
    <li><b>Understand the architecture</b> &rarr; read the <b>Pipeline page</b> (the <i>how it works</i>).</li>
    <li><b>Audit a specific finding</b> &rarr; open <code>task9-review.html</code> (the rule-by-rule deliverable).</li>
    <li><b>Apply the rules</b> &rarr; open the <b>synthesized outputs</b> section (two skills + four CLAUDE.md files).</li>
    <li><b>See what would change in AGENTS.md</b> &rarr; <b>findings/gap-report.html</b> and <b>findings/results-summary.html</b>.</li>
    <li><b>Run it yourself / hand off</b> &rarr; use the <b>Summary Prompt</b> (the <i>how to resume</i>).</li>
  </ul>
</section>

<section>
  <h2>Artifacts (top-level reports)</h2>
  <p>Ordered for a newcomer reading the corpus end-to-end: the prompt summarises what's done, the journey traces how it was done, the dashboards report the findings, and the pipeline page explains the machinery.</p>
  <div class="cards">
{cards_block}
  </div>
</section>

<section>
  <h2>Synthesized outputs (skills + per-area docs)</h2>
  <p>These are the actionable artifacts the entire pipeline exists to produce: two Claude Code skills plus four CLAUDE.md files installed at strategic points in the <code>cli/</code> tree. Editing Rust under <code>cli/</code> auto-loads the cross-cutting skill and the relevant per-area file; the stylistic-conventions skill is loaded on demand or as a sibling.</p>
  <div class="cards">
{synth_cards_block}
  </div>
</section>

<section>
  <h2>Final analysis outputs (findings/ HTML)</h2>
  <p>Three high-level HTML reports that summarise what the pipeline found and what should change. <b>journey-report.html</b> tells the calibration story and complements (does not duplicate) the worktree-root journey log. <b>results-summary.html</b> renders the AGENTS.md before/after diff. <b>gap-report.html</b> is the actionable amendment list.</p>
  <div class="cards">
{final_cards_block}
  </div>
</section>

<section>
  <h2>Rule-level analysis (findings/)</h2>
  <p>Markdown and text artifacts under <code>scripts/pr-analysis/findings/</code> that hold the rule-by-rule substantive output and supporting iteration records. Each now has an HTML rendering with a "Back to index" link; the markdown source is linked underneath as the canonical authored copy.</p>
  <div class="cards">
{findings_cards_block}
  </div>
</section>

<section>
  <h2>Underlying machinery</h2>
  <ul class="tight">
    <li><code>scripts/pr-analysis/</code> &mdash; Pipeline code (~10 ingest/classify/aggregate scripts + <code>lib/</code> modules).</li>
    <li><code>scripts/pr-analysis/data/pr_analysis.db</code> &mdash; SQLite snapshot powering all the HTML reports.</li>
    <li><code>scripts/pr-analysis/lib/md2html.py</code> &mdash; Pure-stdlib markdown-to-HTML helper used to render every markdown artifact in this index.</li>
    <li><code>scripts/pr-analysis/build_dashboard.py</code> &mdash; Regenerates <code>rust-pr-analysis-dashboard-01.html</code>.</li>
    <li><code>scripts/pr-analysis/build_noise_deep_dive.py</code> &mdash; Regenerates <code>rust-pr-analysis-noise-deep-dive-01.html</code>.</li>
    <li><code>scripts/pr-analysis/build_results_html.py</code> &mdash; Regenerates <code>findings/results-summary.html</code>.</li>
    <li><code>scripts/pr-analysis/build_journey_html.py</code> &mdash; Regenerates <code>findings/journey-report.html</code>.</li>
    <li><code>scripts/pr-analysis/build_index_and_pipeline.py</code> &mdash; Regenerates this index, the pipeline page, and every markdown-to-HTML rendering in the manifest.</li>
  </ul>
  <p style="font-size:13px;color:var(--fg-mute);margin-top:10px;">
    Re-running <code>aggregate_findings.py</code> may shift cluster boundaries because greedy single-pass embedding clustering is order-dependent; pin <code>comment_id</code> ordering if you need reproducibility.
  </p>
</section>

<div class="footer">
  Generated {now} &middot; worktree
  <code>{WORKTREE}</code> &middot; latest commit <code>{sha}</code>
</div>

</div>
</body>
</html>
"""


# ---------------------------------------------------------------------------
# Pipeline page (unchanged content, kept inline)
# ---------------------------------------------------------------------------


def svg_pipeline_flow() -> str:
    stages = [
        ("GitHub API", "gh CLI + GraphQL", "#5a6270"),
        ("Ingest", "gh -> SQLite", "#3b6bd6"),
        ("Classify", "Haiku subagents", "#3b6bd6"),
        ("Aggregate", "MiniLM cluster", "#3b6bd6"),
        ("Synthesize", "Sonnet 4.6", "#3b6bd6"),
        ("Skills + Reports", "MD + HTML", "#2f8a52"),
    ]
    box_w, box_h = 150, 80
    gap = 36
    x0 = 20
    y = 30
    total_w = x0 + len(stages) * box_w + (len(stages) - 1) * gap + 20
    svg = [
        f"<svg width='{total_w}' height='180' xmlns='http://www.w3.org/2000/svg'>",
        "<defs>",
        "<marker id='arrow' viewBox='0 0 10 10' refX='9' refY='5' markerWidth='8' "
        "markerHeight='8' orient='auto-start-reverse'>",
        "<path d='M 0 0 L 10 5 L 0 10 z' fill='#5a6270'/>",
        "</marker>",
        "</defs>",
    ]
    for i, (name, sub, color) in enumerate(stages):
        x = x0 + i * (box_w + gap)
        svg.append(
            f"<rect x='{x}' y='{y}' width='{box_w}' height='{box_h}' rx='6' ry='6' "
            f"fill='#ffffff' stroke='{color}' stroke-width='2'/>"
        )
        svg.append(
            f"<text x='{x + box_w/2}' y='{y + 30}' font-size='14' font-weight='600' "
            f"fill='{color}' text-anchor='middle'>{name}</text>"
        )
        svg.append(
            f"<text x='{x + box_w/2}' y='{y + 52}' font-size='11' "
            f"fill='#5a6270' text-anchor='middle'>{sub}</text>"
        )
    for i in range(len(stages) - 1):
        ax = x0 + (i + 1) * box_w + i * gap
        bx = ax + gap
        ay = y + box_h / 2
        svg.append(
            f"<line x1='{ax}' y1='{ay}' x2='{bx - 2}' y2='{ay}' stroke='#5a6270' "
            f"stroke-width='2' marker-end='url(#arrow)'/>"
        )
    svg.append(
        f"<text x='{total_w/2}' y='150' font-size='11' fill='#5a6270' text-anchor='middle'>"
        "Each stage is idempotent and resumable. Audit invariants run between Ingest and Classify."
        "</text>"
    )
    svg.append("</svg>")
    return "".join(svg)


def svg_subagent_orchestration() -> str:
    svg = ["<svg width='820' height='320' xmlns='http://www.w3.org/2000/svg'>"]
    svg.append(
        "<defs><marker id='arrow2' viewBox='0 0 10 10' refX='9' refY='5' "
        "markerWidth='7' markerHeight='7' orient='auto-start-reverse'>"
        "<path d='M 0 0 L 10 5 L 0 10 z' fill='#5a6270'/></marker></defs>"
    )
    svg.append(
        "<rect x='340' y='20' width='160' height='58' rx='6' ry='6' fill='#ffffff' "
        "stroke='#3b6bd6' stroke-width='2'/>"
    )
    svg.append(
        "<text x='420' y='42' font-size='13' font-weight='600' fill='#3b6bd6' "
        "text-anchor='middle'>classify_via_subagent.py</text>"
    )
    svg.append(
        "<text x='420' y='60' font-size='11' fill='#5a6270' text-anchor='middle'>"
        "prepare &middot; dispatch &middot; ingest</text>"
    )
    subs_x = [40, 200, 360, 520, 680]
    for i, x in enumerate(subs_x):
        svg.append(
            f"<rect x='{x}' y='118' width='100' height='30' rx='4' ry='4' "
            f"fill='#f3f4f6' stroke='#e4e6eb'/>"
        )
        svg.append(
            f"<text x='{x + 50}' y='138' font-size='10' fill='#1c1f24' text-anchor='middle'>"
            f"batch_{i+1}.json</text>"
        )
        svg.append(
            f"<rect x='{x}' y='160' width='100' height='44' rx='6' ry='6' fill='#ffffff' "
            f"stroke='#c98a17' stroke-width='2'/>"
        )
        svg.append(
            f"<text x='{x + 50}' y='180' font-size='11' font-weight='600' fill='#c98a17' "
            f"text-anchor='middle'>Haiku</text>"
        )
        svg.append(
            f"<text x='{x + 50}' y='196' font-size='10' fill='#5a6270' "
            f"text-anchor='middle'>subagent {i+1}</text>"
        )
        svg.append(
            f"<rect x='{x}' y='216' width='100' height='30' rx='4' ry='4' "
            f"fill='#f3f4f6' stroke='#e4e6eb'/>"
        )
        svg.append(
            f"<text x='{x + 50}' y='236' font-size='10' fill='#1c1f24' text-anchor='middle'>"
            f"result_{i+1}.json</text>"
        )
        svg.append(
            f"<line x1='420' y1='78' x2='{x + 50}' y2='116' stroke='#5a6270' stroke-width='1' "
            f"marker-end='url(#arrow2)'/>"
        )
        svg.append(
            f"<line x1='{x + 50}' y1='148' x2='{x + 50}' y2='158' stroke='#5a6270' "
            f"stroke-width='1' marker-end='url(#arrow2)'/>"
        )
        svg.append(
            f"<line x1='{x + 50}' y1='204' x2='{x + 50}' y2='214' stroke='#5a6270' "
            f"stroke-width='1' marker-end='url(#arrow2)'/>"
        )
    svg.append(
        "<rect x='340' y='268' width='160' height='40' rx='6' ry='6' fill='#ffffff' "
        "stroke='#2f8a52' stroke-width='2'/>"
    )
    svg.append(
        "<text x='420' y='292' font-size='12' font-weight='600' fill='#2f8a52' "
        "text-anchor='middle'>classification table</text>"
    )
    for x in subs_x:
        svg.append(
            f"<line x1='{x + 50}' y1='246' x2='420' y2='266' stroke='#5a6270' "
            f"stroke-width='1' marker-end='url(#arrow2)'/>"
        )
    svg.append("</svg>")
    return "".join(svg)


def svg_schema_er() -> str:
    tables = {
        "pr":            ("pr_number, merged_at, author, ...",                170, 30),
        "pr_file":       ("pr_number FK, path",                                400, 30),
        "review_summary":("pr_number FK, reviewer, body",                      630, 30),
        "line_comment":  ("id, pr_number FK, path, body, is_noise, thread_resolved", 170, 140),
        "comment_final_code": ("comment_id FK, snippet",                       400, 140),
        "pr_comment":    ("id, pr_number FK, author, body",                    630, 140),
        "classification":("comment_id FK, taxonomy, rule_statement, was_addressed", 170, 250),
        "reviewer":      ("login, tier, weight",                               630, 250),
        "finding":       ("id, taxonomy, area, scope, evidence_comment_ids",   170, 360),
        "synthesis_log": ("id, prompt_hash, output_path",                      630, 360),
    }
    svg = ["<svg width='870' height='460' xmlns='http://www.w3.org/2000/svg'>"]
    svg.append(
        "<defs><marker id='arrow3' viewBox='0 0 10 10' refX='9' refY='5' "
        "markerWidth='7' markerHeight='7' orient='auto-start-reverse'>"
        "<path d='M 0 0 L 10 5 L 0 10 z' fill='#5a6270'/></marker></defs>"
    )
    coords = {}
    for name, (cols, x, y) in tables.items():
        coords[name] = (x, y, 200, 60)
        svg.append(
            f"<rect x='{x}' y='{y}' width='200' height='60' rx='5' ry='5' fill='#ffffff' "
            f"stroke='#3b6bd6' stroke-width='1.5'/>"
        )
        svg.append(
            f"<rect x='{x}' y='{y}' width='200' height='22' rx='5' ry='5' fill='#e9eef9' "
            f"stroke='#3b6bd6' stroke-width='1.5'/>"
        )
        svg.append(
            f"<text x='{x + 100}' y='{y + 16}' font-size='13' font-weight='600' fill='#1e3a7a' "
            f"text-anchor='middle'>{name}</text>"
        )
        svg.append(
            f"<text x='{x + 8}' y='{y + 40}' font-size='10' fill='#5a6270'>{cols}</text>"
        )
    fks = [
        ("pr_file",              "pr"),
        ("review_summary",       "pr"),
        ("pr_comment",           "pr"),
        ("line_comment",         "pr"),
        ("comment_final_code",   "line_comment"),
        ("classification",       "line_comment"),
        ("finding",              "classification"),
        ("synthesis_log",        "finding"),
    ]
    for child, parent in fks:
        cx, cy, cw, ch = coords[child]
        px, py, pw, ph = coords[parent]
        x1 = cx + cw / 2
        y1 = cy
        x2 = px + pw / 2
        y2 = py + ph
        if py < cy:
            svg.append(
                f"<line x1='{x1}' y1='{y1}' x2='{x2}' y2='{y2}' stroke='#5a6270' "
                f"stroke-width='1' stroke-dasharray='4 3' marker-end='url(#arrow3)'/>"
            )
        else:
            svg.append(
                f"<line x1='{cx + cw/2}' y1='{cy + ch}' x2='{px + pw/2}' y2='{py}' "
                f"stroke='#5a6270' stroke-width='1' stroke-dasharray='4 3' marker-end='url(#arrow3)'/>"
            )
    svg.append("</svg>")
    return "".join(svg)


def render_pipeline(stats: dict[str, int | str], sha: str) -> str:
    css = CSS_PALETTE
    flow_svg = svg_pipeline_flow()
    subagent_svg = svg_subagent_orchestration()
    schema_svg = svg_schema_er()

    ingest_rows = [
        ("ingest_prs.py",              "gh pr list --search merged",           "pr",            "Window + --rust-only filter; --since/--until/--limit"),
        ("ingest_comments.py",         "gh api pulls/:n/comments",             "line_comment",  "Applies noise filter; stores commit_id; UPSERT (not REPLACE)"),
        ("ingest_review_summaries.py", "gh api pulls/:n/reviews",              "review_summary","One row per non-empty review body"),
        ("ingest_pr_comments.py",      "gh api issues/:n/comments",            "pr_comment",    "Top-level conversation; not line-anchored"),
        ("ingest_final_code.py",       "gh api repos/.../contents",            "comment_final_code", "~40-line snippet at merge_commit_sha; cached per file"),
        ("ingest_thread_resolution.py","GraphQL pulls/:n/reviewThreads",       "line_comment",  "Updates thread_resolved + thread_resolved_by columns"),
    ]
    ingest_table = "\n".join(
        f"<tr><td><code>{s}</code></td><td><code>{ep}</code></td><td><code>{tbl}</code></td><td>{notes}</td></tr>"
        for s, ep, tbl, notes in ingest_rows
    )

    table_purpose_rows = [
        ("pr",                  "One row per merged PR; metadata + author + merge timestamp"),
        ("pr_file",             "Files changed in each PR (many-to-many to pr)"),
        ("line_comment",        "Line-anchored review comments; includes is_noise, commit_id, thread_resolved"),
        ("comment_final_code",  "~40-line code snippet at merged-final-state for each comment"),
        ("classification",      "LLM classification per comment: taxonomy, was_addressed, rule_statement, prompt_hash"),
        ("finding",             "Clustered themed rules; references multiple comments via evidence_comment_ids JSON"),
        ("review_summary",      "Bodies of submitted review summaries (the text alongside a review submission)"),
        ("pr_comment",          "Top-level issue conversation thread, not line-anchored"),
        ("reviewer",            "Reviewer tier + weight lookup; seeded by init_db"),
        ("synthesis_log",       "Every Sonnet synthesis call captured for audit: prompt_hash, raw response, output path"),
    ]
    table_purpose_html = "\n".join(
        f"<tr><td><code>{t}</code></td><td>{d}</td></tr>" for t, d in table_purpose_rows
    )

    now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Flox Rust PR Analysis &mdash; Pipeline Architecture</title>
<style>
{css}
</style>
</head>
<body>
<div class="container">

<header class="page-header">
  <h1>Pipeline Architecture &amp; Process</h1>
  <div class="subtitle">
    How the PR analysis pipeline turns merged-PR review history into skill files
    &middot; window <b>{stats['window_start']}</b> &rarr; <b>{stats['window_end']}</b>
  </div>
  <p class="blurb">
    The pipeline mines merged Rust-touching PRs from <code>flox/flox</code>, classifies every
    substantive line-comment with a Haiku subagent, clusters the classifications into
    themed findings, and then synthesizes skill files / CLAUDE.md / gap reports from
    those findings. Every stage is idempotent, file-based, and survives subagent
    failures.
  </p>
  <div class="stat-grid">
    <div class="stat"><div class="label">Stages</div><div class="value">5</div></div>
    <div class="stat"><div class="label">Tables</div><div class="value">10</div></div>
    <div class="stat"><div class="label">Taxonomy entries</div><div class="value">15</div></div>
    <div class="stat"><div class="label">Reviewer tiers</div><div class="value">4</div></div>
    <div class="stat"><div class="label">Audit invariants</div><div class="value">4</div></div>
    <div class="stat"><div class="label">Latest commit</div><div class="value"><code>{sha}</code></div></div>
  </div>
  <p style="margin-top:14px;font-size:13px;"><a href="rust-pr-analysis-index-01.html">&larr; Back to index</a></p>
</header>

<section>
  <h2>1. End-to-end flow</h2>
  <p>Each stage reads from the SQLite DB and writes back to it. The diagram below shows the canonical happy-path direction.</p>
  <div class="chart-wrap">{flow_svg}</div>
</section>

<section>
  <h2>2. Stage-by-stage breakdown</h2>

  <h3>2.1 Ingest</h3>
  <p>Pull raw PR + comment + code data from GitHub via the <code>gh</code> CLI (REST and GraphQL) and populate the SQLite tables.</p>
  <table class="std">
    <thead><tr><th>Script</th><th>API endpoint</th><th>Target table</th><th>Notes</th></tr></thead>
    <tbody>
{ingest_table}
    </tbody>
  </table>
  <p style="margin-top:10px;">Audit step: <code>audit_coverage.py --ingest-only</code> validates ID parity with GitHub before any downstream stage runs.</p>

  <h3>2.2 Classify</h3>
  <p>Convert each substantive line-comment into a structured record:</p>
  <pre style="background:var(--code-bg);padding:10px;border-radius:6px;font-size:12px;">{{ taxonomy, was_addressed, rule_statement, confidence, prompt_hash }}</pre>
  <p>Two paths exist:</p>
  <ul class="tight">
    <li><code>classify_via_subagent.py</code> &mdash; subagent-orchestrated; no <code>ANTHROPIC_API_KEY</code> required because the parent Claude session dispatches Haiku subagents directly. <b>This is the path used in production.</b></li>
    <li><code>classify_comments.py</code> &mdash; legacy direct Anthropic SDK path; requires <code>ANTHROPIC_API_KEY</code>. Kept for reference.</li>
  </ul>

  <h4>Subagent flow</h4>
  <ol class="tight">
    <li><b>prepare</b> mode reads pending non-noise line-comments and writes batches of 15 as <code>/tmp/pilot_classify/batch_N.json</code>.</li>
    <li>Controller dispatches parallel Haiku subagents; each subagent reads exactly one batch file and writes <code>result_N.json</code>.</li>
    <li><b>ingest</b> mode reads the result files, validates each row, computes <code>prompt_hash</code>, and writes into the <code>classification</code> table.</li>
  </ol>
  <div class="chart-wrap">{subagent_svg}</div>

  <h3>2.3 Aggregate</h3>
  <p>Cluster classified comments into themed <code>finding</code> rows using semantic similarity.</p>
  <p>Script: <code>aggregate_findings.py</code></p>
  <ul class="tight">
    <li><b>Embedding clustering:</b> <code>sentence-transformers all-MiniLM-L6-v2</code>, cosine similarity at threshold <b>0.65</b>, greedy single-pass.</li>
    <li><b>AGENTS.md matching:</b> key-token substring overlap &mdash; at least <b>3 distinctive tokens of length &ge; 4</b> appearing inside the same AGENTS.md section.</li>
    <li><b>Finding scope:</b> a finding is <code>cross-cutting</code> when <code>cross_area_count &ge; 2 AND tier1_reviewer_count &ge; 1</code>; otherwise it stays scoped to its dominant area.</li>
  </ul>

  <h3>2.4 Synthesize</h3>
  <p>Turn <code>finding</code> rows into human-readable skill files, CLAUDE.md content, and the gap report.</p>
  <ul class="tight">
    <li>Model: Claude Sonnet 4.6, capped at ~4000 output tokens per call.</li>
    <li>Each call is logged in <code>synthesis_log</code> with <code>prompt_hash</code>, raw response, and output path so synthesis is auditable and reproducible.</li>
    <li>Citations are PR numbers drawn from <code>finding.evidence_pr_numbers</code> JSON arrays; the synthesizer is instructed never to invent PR numbers.</li>
  </ul>
  <p>Outputs:</p>
  <ul class="tight">
    <li><code>.claude/skills/flox-rust-review/SKILL.md</code></li>
    <li><code>.claude/skills/flox-rust-stylistic-conventions/SKILL.md</code></li>
    <li>Three area-specific <code>CLAUDE.md</code> files (commands, models/environment, providers) plus <code>cli/CLAUDE.md</code></li>
    <li><code>scripts/pr-analysis/findings/gap-report.md</code></li>
  </ul>

  <h3>2.5 Visualize</h3>
  <p>Regenerate human-readable HTML reports from the current DB state. All outputs are self-contained &mdash; inline SVG, no JavaScript, no CDN.</p>
  <ul class="tight">
    <li><code>build_dashboard.py</code> &rarr; <code>rust-pr-analysis-dashboard-01.html</code></li>
    <li><code>build_noise_deep_dive.py</code> &rarr; <code>rust-pr-analysis-noise-deep-dive-01.html</code></li>
    <li><code>build_results_html.py</code> &rarr; <code>findings/results-summary.html</code></li>
    <li><code>build_journey_html.py</code> &rarr; <code>findings/journey-report.html</code></li>
    <li><code>build_index_and_pipeline.py</code> &rarr; this page, the index, and every markdown-to-HTML rendering</li>
  </ul>
</section>

<section>
  <h2>3. Database schema</h2>
  <p>Ten SQLite tables; foreign keys with <code>ON DELETE CASCADE</code> where children are bound to a parent's identity.</p>
  <div class="chart-wrap">{schema_svg}</div>
  <table class="std" style="margin-top:14px;">
    <thead><tr><th>Table</th><th>Purpose</th></tr></thead>
    <tbody>
{table_purpose_html}
    </tbody>
  </table>
</section>

<section>
  <h2>4. Configuration knobs</h2>
  <p>The values locked in after three pilot iterations. Changing any of these invalidates the existing <code>classification</code> rows because <code>prompt_hash</code> changes.</p>

  <h3>Reviewer tiers and weights</h3>
  <ul class="tight">
    <li><span class="pill good">T1</span> <code>ysndr</code>, <code>mkenigs</code>, <code>dcarley</code> &mdash; weight <b>3.0</b></li>
    <li><span class="pill warn">T2</span> <code>djsauble</code>, <code>gilmishal</code>, <code>billlevine</code> &mdash; weight <b>2.0</b></li>
    <li><span class="pill neutral">T3</span> all other humans &mdash; weight <b>1.0</b></li>
    <li><span class="pill bad">T4</span> bots (<code>*[bot]</code>, <code>Copilot</code>) &mdash; weight <b>0.0</b>, excluded</li>
  </ul>

  <h3>Hot areas</h3>
  <ul class="tight">
    <li><code>cli/flox/src/commands/</code></li>
    <li><code>cli/flox-rust-sdk/src/models/environment/</code></li>
    <li><code>cli/flox-rust-sdk/src/providers/</code></li>
  </ul>

  <h3>Noise filter patterns</h3>
  <ul class="tight">
    <li>URL-only bodies</li>
    <li>Suggestion-block-only (just a <code>```suggestion</code> diff)</li>
    <li>lgtm / thanks / emoji-only acknowledgments</li>
    <li>Praise/nit prefix with body &le; 40 characters</li>
  </ul>

  <h3>Taxonomy (15 entries, seeded from AGENTS.md sections)</h3>
  <p>
    <code>error-handling</code>, <code>provider-traits</code>, <code>type-safety</code>,
    <code>user-facing-messages</code>, <code>naming</code>, <code>testing</code>,
    <code>imports</code>, <code>manifest-usage</code>, <code>deprecated-patterns</code>,
    <code>logging-tracing</code>, <code>formatting-style</code>, <code>control-flow</code>,
    <code>semantic-correctness</code>, <code>ld-floxlib</code>, <code>panic-discipline</code>,
    <code>other</code>
  </p>

  <h3>Other</h3>
  <ul class="tight">
    <li><b>Cluster threshold:</b> MiniLM cosine <b>0.65</b></li>
    <li><b>AGENTS.md matching:</b> key-token substring, &ge; 3 distinctive tokens, each &ge; 4 chars</li>
    <li><b>Batch size:</b> 15 comments per Haiku subagent (Iter 3 calibration)</li>
    <li><b>Cross-cutting requirement:</b> <code>cross_area_count &ge; 2 AND tier1_reviewer_count &ge; 1</code></li>
    <li><b>Confidence formula:</b> <code>0.4 &middot; tier_signal + 0.2 &middot; min(evd/5, 1) + 0.2 &middot; min(cross_area/3, 1) + 0.2 &middot; acceptance_rate</code></li>
  </ul>
</section>

<section>
  <h2>5. Subagent orchestration</h2>
  <p><b>Why the subagent path exists:</b> the analysis already runs inside a Claude Code session, so requiring users to also manage an <code>ANTHROPIC_API_KEY</code> would be an unnecessary integration burden. The subagent path piggybacks on the existing Claude entitlement.</p>
  <p><b>How it works:</b> the parent dispatches background <code>Agent(model: &quot;haiku&quot;)</code> calls per batch file. Each subagent reads exactly one <code>batch_N.json</code> from <code>/tmp/pilot_classify</code> and writes exactly one <code>result_N.json</code>. Failed subagents leave their batch file untouched and can be retried.</p>
  <p><b>Reproducibility:</b> <code>prompt_hash = SHA256(SYSTEM_PROMPT + &quot;\\n---\\n&quot; + taxonomy_block)</code> is stored per <code>classification</code> row. Classifications from different prompt or taxonomy versions are distinguishable in the DB so synthesis can choose a coherent subset.</p>
  <p><b>Coordination is file-based, not in-memory:</b> subagent failures don't lose work; the controller can rescan <code>/tmp/pilot_classify</code> on every run.</p>
  <div class="chart-wrap">{subagent_svg}</div>
</section>

<section>
  <h2>6. Invariants enforced by <code>audit_coverage.py</code></h2>
  <ol class="tight">
    <li><b>ID parity</b> &mdash; the set of comment IDs in the DB matches what GitHub reports per PR. Catches a partial ingest or a deleted comment.</li>
    <li><b>Snippet coverage</b> &mdash; every <code>line_comment</code> has a <code>comment_final_code</code> row (the snippet itself may be NULL if the file was deleted, but the row must exist).</li>
    <li><b>Classification coverage</b> &mdash; every non-noise, non-bot <code>line_comment</code> has at least one <code>classification</code> row. Skipped under <code>--ingest-only</code>.</li>
    <li><b>Area mapping</b> &mdash; no <code>cli/*</code> path falls into <code>area='other'</code>. A failure indicates a missing prefix in <code>lib/areas.py</code>.</li>
  </ol>
</section>

<section>
  <h2>7. Known pitfalls and lessons</h2>
  <ol class="tight">
    <li><b>INSERT OR REPLACE cascades.</b> SQLite implements <code>REPLACE</code> as DELETE + INSERT, which triggers <code>ON DELETE CASCADE</code> on children. <code>ingest_comments</code> originally lost every <code>classification</code> on each re-ingest. Fix: <code>INSERT &hellip; ON CONFLICT(id) DO UPDATE SET &hellip;</code>.</li>
    <li><b>Subagent self-doubt.</b> Haiku occasionally pre-emptively claims a <code>Read</code> might exceed token limits and refuses. Dropping the batch size from 30 to 15 eliminated the trigger entirely.</li>
    <li><b>Heuristic calibration swings.</b> AGENTS.md matching went <span class="pill bad">100%</span> &rarr; <span class="pill bad">0%</span> &rarr; <span class="pill good">73%</span> across three iterations of &ldquo;the same idea&rdquo; expressed differently (substring &rarr; whole-word &rarr; key-token &ge; 3 of len &ge; 4). The middle iteration was as broken as the first; only the third was defensible.</li>
    <li><b>Window sampling matters.</b> A 15-PR recent-window pilot showed 52% activation rate on a key heuristic &mdash; entirely because of one fix-series concentrated in that window. Always validate calibration on a different historical window before scaling to the full corpus.</li>
    <li><b>Stylistic rules are under-codified.</b> 33% of stylistic-taxonomy rules are not in AGENTS.md, vs 27% overall. Reviewers enforce them but they live as tribal knowledge &mdash; which is why a dedicated <code>flox-rust-stylistic-conventions</code> skill is the right next deliverable.</li>
  </ol>
</section>

<div class="footer">
  Generated {now} &middot; latest commit <code>{sha}</code> &middot;
  <a href="rust-pr-analysis-index-01.html">Back to index</a>
</div>

</div>
</body>
</html>
"""


def main() -> None:
    stats = db_stats()
    sha = git_short_sha()
    commit_count = commits_in_build()

    # 1. Convert all markdown entries first.
    print("Converting markdown entries...")
    results = convert_md_entries(md_manifest())
    for path, size in results:
        print(f"  {path.relative_to(WORKTREE)} ({size:,} bytes)")

    # 2. Render index + pipeline.
    index_html = render_index(stats, sha, commit_count)
    pipeline_html = render_pipeline(stats, sha)

    INDEX_PATH.write_text(index_html, encoding="utf-8")
    PIPELINE_PATH.write_text(pipeline_html, encoding="utf-8")

    print(f"wrote {INDEX_PATH.relative_to(WORKTREE)} ({INDEX_PATH.stat().st_size} bytes)")
    print(f"wrote {PIPELINE_PATH.relative_to(WORKTREE)} ({PIPELINE_PATH.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
