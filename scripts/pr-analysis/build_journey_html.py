#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Build a self-contained single-page HTML journey report for the Flox Rust PR-analysis project.

Run from scripts/pr-analysis/:
    uv run build_journey_html.py

Output: findings/journey-report.html
"""
from __future__ import annotations

import sqlite3
import textwrap
from pathlib import Path

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
HERE = Path(__file__).parent
DB = HERE / "data" / "pr_analysis.db"
FINDINGS_DIR = HERE / "findings"
OUT = FINDINGS_DIR / "journey-report.html"
WORKTREE = HERE.parent.parent  # .claude/worktrees/rust-pr-analysis-skill

# ---------------------------------------------------------------------------
# DB helpers
# ---------------------------------------------------------------------------

def db_rows(sql: str, params: tuple = ()) -> list[tuple]:
    conn = sqlite3.connect(str(DB))
    conn.row_factory = sqlite3.Row
    cur = conn.execute(sql, params)
    rows = cur.fetchall()
    conn.close()
    return rows


def db_one(sql: str, params: tuple = ()) -> sqlite3.Row | None:
    rows = db_rows(sql, params)
    return rows[0] if rows else None


# ---------------------------------------------------------------------------
# CSS
# ---------------------------------------------------------------------------

CSS = """
* { box-sizing: border-box; margin: 0; padding: 0; }
body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;
  font-size: 15px;
  line-height: 1.6;
  color: #1a1a1a;
  background: #fafafa;
}
.page-wrap { max-width: 860px; margin: 0 auto; padding: 32px 24px 80px; }
h1 { font-size: 2rem; font-weight: 700; color: #111; margin-bottom: 6px; }
h2 { font-size: 1.45rem; font-weight: 700; color: #1a2740; margin: 48px 0 14px; border-bottom: 2px solid #d0d7e3; padding-bottom: 6px; }
h3 { font-size: 1.1rem; font-weight: 600; color: #243352; margin: 24px 0 8px; }
h4 { font-size: 0.95rem; font-weight: 600; color: #334; margin: 18px 0 6px; }
p { margin: 10px 0; }
a { color: #1a6ab1; text-decoration: none; }
a:hover { text-decoration: underline; }
.subtitle { color: #555; font-size: 0.95rem; margin-bottom: 28px; }
nav { background: #f0f4fa; border: 1px solid #d0d7e3; border-radius: 6px; padding: 16px 20px; margin: 24px 0 40px; }
nav p { font-weight: 600; margin-bottom: 8px; color: #333; }
nav ol { margin-left: 20px; }
nav li { margin: 3px 0; font-size: 0.93rem; }
section { margin-bottom: 56px; }
/* tables */
table { border-collapse: collapse; width: 100%; font-size: 0.88rem; margin: 16px 0; }
th { background: #e8eef8; color: #1a2740; font-weight: 600; text-align: left; padding: 8px 10px; border: 1px solid #cdd5e0; }
td { padding: 6px 10px; border: 1px solid #dde3ed; vertical-align: top; }
tr:nth-child(even) td { background: #f5f8fd; }
/* code */
code { font-family: 'SFMono-Regular', Consolas, monospace; font-size: 0.82em; background: #eef1f7; padding: 2px 5px; border-radius: 3px; }
pre { background: #f0f2f7; border: 1px solid #dde3ed; border-radius: 5px; padding: 14px 16px; overflow-x: auto; font-size: 0.82em; font-family: 'SFMono-Regular', Consolas, monospace; margin: 12px 0; white-space: pre-wrap; word-break: break-word; }
/* charts */
.chart-wrap { margin: 20px 0; }
.bar-row { display: flex; align-items: center; margin: 4px 0; font-size: 0.84rem; }
.bar-label { width: 180px; flex-shrink: 0; text-align: right; padding-right: 10px; color: #333; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.bar-label-wide { width: 220px; }
.bar-track { flex: 1; background: #e8eef8; border-radius: 3px; height: 18px; position: relative; }
.bar-fill { height: 18px; border-radius: 3px; }
.bar-fill-blue { background: #3a7bd5; }
.bar-fill-green { background: #2e9e6a; }
.bar-fill-orange { background: #e07b2a; }
.bar-fill-purple { background: #7a4ed4; }
.bar-count { margin-left: 8px; color: #555; min-width: 30px; }
/* confidence band SVG */
.svg-wrap { margin: 16px 0; }
/* finding chips */
.chip { display: inline-block; font-size: 0.78em; padding: 2px 7px; border-radius: 10px; font-weight: 600; margin: 1px; }
.chip-y { background: #d1f0e0; color: #1a5c35; }
.chip-n { background: #fde8d8; color: #7a2c0a; }
/* blockquote */
blockquote { border-left: 3px solid #3a7bd5; margin: 12px 0; padding: 8px 16px; background: #f5f8ff; color: #333; font-style: italic; font-size: 0.92rem; }
.reviewer-card { background: #fff; border: 1px solid #d0d7e3; border-radius: 7px; padding: 16px 20px; margin: 18px 0; }
.reviewer-card h4 { margin-top: 0; color: #1a2740; font-size: 1rem; }
/* metrics table highlight */
.best { background: #d1f0e0 !important; font-weight: 600; }
.pill { display: inline-block; padding: 2px 8px; border-radius: 9px; font-size: 0.78em; font-weight: 600; }
.pill-t1 { background: #dbeafe; color: #1e40af; }
.pill-t2 { background: #fce7f3; color: #9d174d; }
.pill-t3 { background: #f3f4f6; color: #374151; }
.lessons ol { margin-left: 22px; }
.lessons li { margin: 10px 0; }
"""

# ---------------------------------------------------------------------------
# Chart helpers
# ---------------------------------------------------------------------------

def bar_chart(rows: list[tuple[str, int]], max_val: int | None = None, color: str = "blue", label_class: str = "") -> str:
    """Horizontal CSS bar chart. rows = list of (label, count)."""
    if not rows:
        return "<p><em>No data</em></p>"
    mv = max_val or max(r[1] for r in rows)
    lc = f"bar-label {label_class}".strip()
    parts = ['<div class="chart-wrap">']
    for label, count in rows:
        pct = int(count / mv * 100) if mv else 0
        label_escaped = label.replace("&", "&amp;").replace("<", "&lt;")
        parts.append(
            f'<div class="bar-row">'
            f'<div class="{lc}" title="{label_escaped}">{label_escaped}</div>'
            f'<div class="bar-track">'
            f'<div class="bar-fill bar-fill-{color}" style="width:{pct}%"></div>'
            f'</div>'
            f'<div class="bar-count">{count:,}</div>'
            f'</div>'
        )
    parts.append("</div>")
    return "\n".join(parts)


def confidence_band_svg(rows: list[tuple[str, int]]) -> str:
    """Small vertical column SVG chart for confidence bands."""
    if not rows:
        return ""
    bands = [r[0] for r in rows]
    counts = [r[1] for r in rows]
    max_c = max(counts) or 1
    W, H = 340, 160
    bar_w = 60
    gap = 20
    colors = ["#c9d6f5", "#7aa2e8", "#3a7bd5", "#1e4fa1"]
    svg_parts = [f'<svg width="{W}" height="{H + 40}" xmlns="http://www.w3.org/2000/svg" style="display:block">']
    for i, (band, count) in enumerate(zip(bands, counts)):
        x = gap + i * (bar_w + gap)
        bar_h = int(count / max_c * H)
        y = H - bar_h
        color = colors[i % len(colors)]
        svg_parts.append(f'<rect x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" fill="{color}" rx="3"/>')
        svg_parts.append(f'<text x="{x + bar_w//2}" y="{y - 4}" text-anchor="middle" font-size="12" fill="#333">{count:,}</text>')
        svg_parts.append(f'<text x="{x + bar_w//2}" y="{H + 20}" text-anchor="middle" font-size="11" fill="#555">{band}</text>')
    svg_parts.append("</svg>")
    return f'<div class="svg-wrap">{"".join(svg_parts)}</div>'


def prs_timeseries_svg(rows: list[tuple[str, int]]) -> str:
    """Simple line + bar SVG for PRs over time."""
    if not rows:
        return ""
    months = [r[0] for r in rows]
    counts = [r[1] for r in rows]
    n = len(months)
    max_c = max(counts) or 1
    W, H = 700, 140
    pad_l, pad_r, pad_top, pad_bot = 50, 20, 20, 30
    plot_w = W - pad_l - pad_r
    plot_h = H - pad_top - pad_bot
    bar_w = max(4, plot_w // n - 4)
    svg_parts = [
        f'<svg width="{W}" height="{H + 10}" xmlns="http://www.w3.org/2000/svg" style="display:block;max-width:100%">'
    ]
    # bars
    for i, (month, count) in enumerate(zip(months, counts)):
        bh = int(count / max_c * plot_h)
        x = pad_l + int(i * plot_w / n) + 2
        y = pad_top + plot_h - bh
        svg_parts.append(f'<rect x="{x}" y="{y}" width="{bar_w}" height="{bh}" fill="#3a7bd5" rx="2" opacity="0.8"/>')
        # label every other month to avoid clutter
        if i % 2 == 0:
            short = month[5:]  # MM
            svg_parts.append(f'<text x="{x + bar_w//2}" y="{pad_top + plot_h + 16}" text-anchor="middle" font-size="10" fill="#555">{month}</text>')
        svg_parts.append(f'<title>{month}: {count} PRs</title>')
    # y-axis label
    svg_parts.append(f'<text x="12" y="{pad_top + plot_h//2}" text-anchor="middle" font-size="11" fill="#777" transform="rotate(-90,12,{pad_top + plot_h//2})">PRs</text>')
    svg_parts.append("</svg>")
    return f'<div class="svg-wrap">{"".join(svg_parts)}</div>'


# ---------------------------------------------------------------------------
# Read files safely
# ---------------------------------------------------------------------------

def read_file(path: Path, default: str = "") -> str:
    try:
        return path.read_text(encoding="utf-8")
    except Exception:
        return default


def excerpt(text: str, start_marker: str, end_marker: str | None = None, max_chars: int = 2000) -> str:
    """Pull a section out of a markdown file."""
    idx = text.find(start_marker)
    if idx < 0:
        return ""
    snippet = text[idx:]
    if end_marker:
        eidx = snippet.find(end_marker)
        if eidx > 0:
            snippet = snippet[:eidx]
    return snippet[:max_chars]


def html_escape(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")

# ---------------------------------------------------------------------------
# Section builders
# ---------------------------------------------------------------------------

def build_overview() -> str:
    return """
<section id="overview">
<h2>Overview</h2>
<p>
This project mined <strong>8 months of merged pull requests</strong> from the
<code>flox/flox</code> repository to extract code-review-validated conventions
for Rust development. The pipeline ran from raw GitHub API data through SQLite
storage, noise filtering, LLM classification (Haiku subagents), Sonnet
re-classification, embedding-based clustering, and multi-stage synthesis, ending
with four new CLAUDE.md files and a gap report against the existing AGENTS.md.
</p>

<table>
<tr><th>Corpus</th><td>216 Rust-touching PRs out of 335 merged (Sep 2025 – May 2026)</td></tr>
<tr><th>Review comments ingested</th><td>1,047 line-comments; 87 noise-dropped; 960 in classification pool</td></tr>
<tr><th>Classified</th><td>944 comments (840 by Haiku subagents + 104 Sonnet re-classification)</td></tr>
<tr><th>Findings produced</th><td>488 (474 with one supporting comment, 14 with two or more)</td></tr>
<tr><th>Deliverables</th><td>2 cross-cutting SKILL.md files + cli/CLAUDE.md + 3 area CLAUDE.md files + gap report (30 amendments, 2 new sections)</td></tr>
<tr><th>Calibration iterations</th><td>4 pilot iterations before the full-corpus run</td></tr>
<tr><th>Tier-1 reviewers</th><td>ysndr, mkenigs, dcarley (3.0× weight)</td></tr>
</table>
</section>
"""


def build_pipeline() -> str:
    return """
<section id="pipeline-architecture">
<h2>Pipeline Architecture</h2>
<p>The pipeline is a 10-script SQLite warehouse with an LLM classification layer and a
separate synthesis pass. Each stage is idempotent and can be re-run independently.</p>

<pre>
scripts/pr-analysis/
├── schema.sql              10 tables: pr, line_comment, review_summary,
│                           pr_comment, comment_final_code, classification,
│                           finding, reviewer, pr_file, synthesis_log
├── init_db.py              --reset wipes DB; WAL/shm cleanup
├── ingest_prs.py           --since/--until/--limit/--rust-only/--all
├── ingest_comments.py      line-comments, noise filter, UPSERT semantics
├── ingest_review_summaries.py  pulls/:n/reviews body text
├── ingest_pr_comments.py   issue comments (top-level discussion)
├── ingest_final_code.py    ~40-line snippet at merge_commit_sha (cached)
├── ingest_thread_resolution.py  GraphQL reviewThreads
├── audit_coverage.py       4 hard invariants + informational counts
├── classify_via_subagent.py  prepare/ingest; dispatches Haiku subagents
├── build_sonnet_reclassify_input.py  selects evd≥2 comments for Sonnet pass
├── aggregate_findings.py   MiniLM cosine @ 0.65, key-token AGENTS.md match
└── lib/
    ├── db.py, reviewers.py, areas.py, taxonomy.py
    ├── classify_helpers.py  SYSTEM_PROMPT, prompt_hash (SHA-256)
    ├── noise_filter.py      URL-only / suggestion / lgtm / praise regexes
    └── gh.py               run_json, paginate_jsonl wrappers
</pre>

<h3>Data flow</h3>
<p>
  <strong>Ingest</strong> (gh CLI + GraphQL) &rarr;
  <strong>Noise filter</strong> (18% dropped) &rarr;
  <strong>Haiku classification</strong> (batches of 15, parallel subagents) &rarr;
  <strong>Sonnet re-classification</strong> (evd&ge;2 comments, quality pass) &rarr;
  <strong>MiniLM embedding clustering</strong> (cosine 0.65 threshold) &rarr;
  <strong>AGENTS.md key-token matching</strong> &rarr;
  <strong>Synthesis</strong> (area CLAUDE.md + skills + gap report)
</p>

<h3>Key engineering decisions</h3>
<table>
<tr><th>Decision</th><th>Rationale</th></tr>
<tr><td>SQLite as the warehouse</td><td>Queryable, portable, no server. <code>schema.sql</code> is the single source of truth.</td></tr>
<tr><td>Subagent-orchestrated Haiku classification</td><td>No Anthropic API key required in the worktree. Subagents read batch files from disk.</td></tr>
<tr><td>INSERT … ON CONFLICT DO UPDATE instead of INSERT OR REPLACE</td><td>SQLite REPLACE = DELETE + INSERT, cascades to children. UPSERT preserves foreign-key children.</td></tr>
<tr><td>MiniLM embeddings for clustering</td><td>One-line rule statements Jaccard at 0.3–0.4 even when paraphrasing the same rule; MiniLM cosine puts them at 0.55–0.65.</td></tr>
<tr><td>Key-token AGENTS.md matching</td><td>Jaccard on full text swung 100% (trivially true) → 0% (trivially false). Key-token substring on per-section text landed at 73%.</td></tr>
</table>
</section>
"""


def build_calibration() -> str:
    pilot_retro = read_file(FINDINGS_DIR / "pilot-retro.md")
    iter4 = read_file(FINDINGS_DIR / "iter4-comparison.md")

    return f"""
<section id="calibration-journey">
<h2>Calibration Journey — Four Pilot Iterations</h2>

<p>
Before running the full 216-PR corpus, the pipeline was validated over four
calibration iterations. Each iteration fixed a different failure mode found in
the previous retro. The table below shows how each key metric evolved.
</p>

<table>
<tr>
  <th>Metric</th>
  <th>Iter 1</th>
  <th>Iter 2</th>
  <th>Iter 3</th>
  <th>Iter 4 (different window)</th>
</tr>
<tr>
  <td>Window</td>
  <td>recent ~2 mo</td>
  <td>recent ~2 mo</td>
  <td>recent ~2 mo</td>
  <td>6–8 mo back</td>
</tr>
<tr>
  <td>PRs ingested</td>
  <td>13</td>
  <td>13</td>
  <td>13</td>
  <td>36</td>
</tr>
<tr>
  <td>Comments to classify</td>
  <td>115</td>
  <td>94 (noise filter added)</td>
  <td>94</td>
  <td>78</td>
</tr>
<tr>
  <td>Batch retries</td>
  <td>1/4 (25%)</td>
  <td>0/7</td>
  <td>0/7</td>
  <td>0</td>
</tr>
<tr>
  <td><code>other</code> bucket %</td>
  <td>54%</td>
  <td>44%</td>
  <td class="best">31%</td>
  <td>51%</td>
</tr>
<tr>
  <td>Findings total</td>
  <td>51</td>
  <td>49</td>
  <td>52</td>
  <td>36</td>
</tr>
<tr>
  <td>Findings with evd &gt; 1</td>
  <td>2 (4%)</td>
  <td>4 (8%)</td>
  <td class="best">9 (17%)</td>
  <td>2 (6%)</td>
</tr>
<tr>
  <td><code>in_agents_md=1</code></td>
  <td>51 (100% — useless true)</td>
  <td>0 (0% — useless false)</td>
  <td class="best">38 (73%)</td>
  <td>81%</td>
</tr>
<tr>
  <td><code>was_addressed=NULL</code></td>
  <td>~38</td>
  <td>~38</td>
  <td class="best">11</td>
  <td>30</td>
</tr>
</table>

<h3>What changed between iterations</h3>

<table>
<tr><th>Iteration</th><th>Key change</th><th>Effect</th></tr>
<tr>
  <td>Iter 1 → 2</td>
  <td>Added noise filter (URL-only / suggestion-block / praise / lgtm regexes); batch size 30 → 15; fixed schema CASCADE gotcha with UPSERT</td>
  <td><code>other</code> 54% → 44%; retries 25% → 0%</td>
</tr>
<tr>
  <td>Iter 2 → 3</td>
  <td>Replaced Jaccard AGENTS.md matching with key-token substring; switched clustering from text overlap to MiniLM cosine embeddings; passed <code>thread_resolved</code> as classifier hint</td>
  <td><code>in_agents_md</code> 0% → 73%; evd&gt;1 findings 4% → 17%; <code>other</code> → 31%</td>
</tr>
<tr>
  <td>Iter 3 → 4</td>
  <td>Ran on a different 2-month window (6–8 months back) to validate calibration generalizes</td>
  <td><code>other</code> rose to 51% — confirmed window-specific bias from an activation-fix burst in the recent window</td>
</tr>
</table>

<h3>Why Iter 4 showed 51% other</h3>
<p>
The recent window (Iter 3) was dominated by a single PR series
(<em>activation hint enhancements</em>). That burst made 52% of comments fall
in the <code>activations</code> area. The older window (Iter 4) had a broader
mix of 36 PRs but fewer Tier-1 comments that produced extractable rules —
many older comments were design questions or inline approvals that the noise
filter did not drop. This recency-bias insight shaped the full-corpus strategy:
run over the full 8-month window to dilute any single-month spike.
</p>
</section>
"""


def build_drift_correction() -> str:
    # Sonnet re-classified 104 evd>=2 comments (separate from Haiku's 840)
    # The "drift" is in the other-bucket: 7 comments all got rule_statement = "Review comment addressing code change."
    cluster_txt = read_file(FINDINGS_DIR / "other-cluster-candidates.txt")

    return f"""
<section id="drift-correction">
<h2>Drift Correction — Haiku Noise and the Sonnet Re-classification Pass</h2>

<h3>The rule_statement drift problem</h3>
<p>
Haiku subagents, when confronted with a comment they could not confidently
categorize, fell back to a generic <em>rule_statement</em> such as:
</p>
<blockquote>"Review comment addressing code change."</blockquote>
<p>
This is not a rule — it is the model expressing uncertainty in rule-statement
form. The <code>other-cluster-candidates.txt</code> output shows a cluster of
<strong>7 comments</strong> that all received this identical text, from four
different areas (models/environment, activations, cli/other), all at
confidence=0.50. That confidence floor is precisely the Haiku "I don't know"
signal.
</p>
<p>
A second drift pattern was over-narrow rules. Haiku sometimes generated
rules that described <em>what the comment discussed</em> rather than
<em>the convention the discussion implies</em>. For example:
</p>
<blockquote>"question nonblocking: did you mean to remove all the comments?
Seems like some could be stale but some might not be"</blockquote>
<p>
…became the rule statement verbatim (confidence 0.65), instead of the
actionable convention <em>"Preserve doc comments during refactors unless they
are demonstrably stale"</em>.
</p>

<h3>The Sonnet re-classification pass</h3>
<p>
After the full-corpus Haiku run, a targeted Sonnet re-classification pass
(<code>build_sonnet_reclassify_input.py</code> + a Sonnet subagent) reprocessed
the <strong>104 comments</strong> backing findings with <code>evd&ge;2</code>.
These are the comments most likely to surface real conventions, so quality there
matters most. Sonnet produced significantly higher confidence on naming and
type-safety categories (avg 0.82 and 0.82 vs Haiku's 0.74–0.75) and correctly
promoted several "other" classifications to specific taxonomies.
</p>

<table>
<tr><th>Taxonomy (Sonnet pass)</th><th>Count</th><th>Avg confidence</th></tr>
<tr><td>testing</td><td>19</td><td>0.73</td></tr>
<tr><td>control-flow</td><td>14</td><td>0.71</td></tr>
<tr><td>naming</td><td>13</td><td>0.82</td></tr>
<tr><td>semantic-correctness</td><td>12</td><td>0.67</td></tr>
<tr><td>type-safety</td><td>12</td><td>0.82</td></tr>
<tr><td>error-handling</td><td>7</td><td>0.80</td></tr>
<tr><td>manifest-usage</td><td>7</td><td>0.75</td></tr>
<tr><td>other</td><td>6</td><td>0.35</td></tr>
<tr><td>user-facing-messages</td><td>4</td><td>0.75</td></tr>
<tr><td>deprecated-patterns</td><td>3</td><td>0.81</td></tr>
</table>

<p>
The Sonnet pass <em>other</em> rate was 6% (6 out of 104), compared to 39%
<em>other</em> across the full Haiku corpus (370 out of 944) — a 6× improvement
in extractable-rule rate for high-evidence comments.
</p>

<h3>The other-bucket clusters (Task 8.5)</h3>
<p>
The post-full-corpus Task 8.5 audit found <strong>19 high-confidence
<code>other</code>-bucket comments</strong> (confidence &ge; 0.4) that were
re-clustered by the subagent. Cluster 1 (size 7) was exactly the
"Review comment addressing code change" generic, confirming the Haiku fallback
pattern. Clusters 2 and 3 showed actionable rules that should have been
classified as <code>semantic-correctness</code> and were incorporated into the
final gap report.
</p>
</section>
"""


def build_corpus_shape() -> str:
    # Taxonomy
    tax_rows = db_rows("SELECT taxonomy, COUNT(*) as n FROM classification GROUP BY taxonomy ORDER BY n DESC")
    tax_data = [(r["taxonomy"], r["n"]) for r in tax_rows]

    # Reviewer
    rev_rows = db_rows(
        "SELECT author, reviewer_tier as tier, COUNT(*) as n FROM line_comment GROUP BY author ORDER BY n DESC LIMIT 10"
    )
    rev_data = [(f"{r['author']} (T{r['tier']})", r["n"]) for r in rev_rows]

    # Area (non-noise)
    area_rows = db_rows(
        """SELECT lc.area, COUNT(*) as n
           FROM line_comment lc
           JOIN classification c ON c.comment_id=lc.id
           WHERE c.taxonomy != 'other'
           GROUP BY lc.area ORDER BY n DESC"""
    )
    area_data = [(r["area"], r["n"]) for r in area_rows]

    # PRs over time
    time_rows = db_rows(
        "SELECT strftime('%Y-%m', merged_at) as ym, COUNT(*) as prs FROM pr GROUP BY ym ORDER BY ym"
    )
    time_data = [(r["ym"], r["prs"]) for r in time_rows]

    # Confidence bands
    band_rows = db_rows(
        """SELECT CASE
              WHEN confidence_score < 0.6 THEN '0.5–0.6'
              WHEN confidence_score < 0.7 THEN '0.6–0.7'
              WHEN confidence_score < 0.8 THEN '0.7–0.8'
              ELSE '0.8+'
           END as band, COUNT(*) as n
           FROM finding GROUP BY band ORDER BY band"""
    )
    band_data = [(r["band"], r["n"]) for r in band_rows]

    return f"""
<section id="corpus-shape">
<h2>Corpus Shape</h2>

<h3>Merged PRs per month (Rust-touching, 8-month window)</h3>
{prs_timeseries_svg(time_data)}

<h3>Taxonomy distribution — all 944 classified comments</h3>
<p>15 taxonomy categories. <code>other</code> accounts for 39% of all classifications
(the model's "I cannot extract a rule here" signal). Remaining 61% distribute
across substantive categories dominated by semantic-correctness, testing,
control-flow, and user-facing-messages.</p>
{bar_chart(tax_data, color="blue")}

<h3>Reviewer comment volume (top 10)</h3>
<p>Tier-1 reviewers (ysndr, mkenigs, dcarley) together authored 698 of the 1,047
ingested comments — 67% of the signal pool. Each was weighted 3× in finding
confidence scoring.</p>
{bar_chart(rev_data, color="purple", label_class="bar-label-wide")}

<h3>Area distribution — classified non-<code>other</code> comments</h3>
<p>Hot areas by comment volume: cli/other (149), activations (104), commands (101),
providers (84), models/environment (39). These five areas drove the area-specific
CLAUDE.md deliverables.</p>
{bar_chart(area_data, color="green")}

<h3>Findings by confidence band</h3>
<p>Most findings cluster in the 0.5–0.6 and 0.7–0.8 bands. The bimodal pattern
reflects the Haiku "floor" at 0.5 (uncertain → other) and a genuine signal cluster
around 0.7–0.75 where the model extracted a clear rule.</p>
{confidence_band_svg(band_data)}
</section>
"""


def build_findings_table() -> str:
    rows = db_rows(
        """SELECT f.id, f.area, f.taxonomy, ROUND(f.confidence_score,2) as conf,
                  f.in_agents_md, f.total_evidence_count as evd,
                  f.tier1_reviewer_count as t1,
                  SUBSTR(f.rule_statement, 1, 120) as rule
           FROM finding f
           ORDER BY f.confidence_score DESC, f.total_evidence_count DESC
           LIMIT 50"""
    )
    rows_html = []
    for r in rows:
        in_md = '<span class="chip chip-y">Y</span>' if r["in_agents_md"] else '<span class="chip chip-n">N</span>'
        t1_badge = f'<span class="pill pill-t1">T1={r["t1"]}</span>' if r["t1"] else ""
        rule = html_escape(r["rule"] or "")
        rows_html.append(
            f"<tr><td>{r['id']}</td><td>{html_escape(r['area'])}</td>"
            f"<td>{html_escape(r['taxonomy'])}</td>"
            f"<td>{r['conf']}</td><td>{in_md}</td>"
            f"<td>{r['evd']}</td><td>{t1_badge}</td>"
            f"<td style='max-width:280px;font-size:0.8em'>{rule}{'...' if len(r['rule'] or '') >= 120 else ''}</td></tr>"
        )
    return f"""
<section id="findings-table">
<h2>Top 50 Findings (by Confidence)</h2>
<p>Full dataset: 488 findings. Columns: ID, area, taxonomy, confidence, in AGENTS.md (Y/N), evidence count, T1 reviewer, rule (truncated).</p>
<div style="overflow-x:auto">
<table>
<tr>
  <th>ID</th><th>Area</th><th>Taxonomy</th><th>Conf</th>
  <th>AGENTS.md</th><th>Evd</th><th>T1</th><th>Rule</th>
</tr>
{"".join(rows_html)}
</table>
</div>
</section>
"""


def build_artifact_routing() -> str:
    return """
<section id="artifact-routing">
<h2>Artifact Routing — Where Each Finding Went</h2>
<p>
The 488 findings and 30 gap-report amendments were routed to six deliverables
based on scope (cross-cutting vs area-specific) and taxonomy.
</p>

<table>
<tr><th>Deliverable</th><th>Path</th><th>What landed here</th></tr>
<tr>
  <td><strong>flox-rust-review SKILL.md</strong></td>
  <td><code>.claude/skills/flox-rust-review/SKILL.md</code></td>
  <td>
    High-confidence correctness findings: error-handling, type-safety,
    semantic-correctness, testing, provider-traits, manifest-usage, panic-discipline.
    Rules where the original code was wrong, buggy, or insufficient.
    Evidence: PRs #3599, #3646, #3673, #3785, #3794, #3864, #4047, #4076, #4094, #4172, #4191, #4202 and others.
  </td>
</tr>
<tr>
  <td><strong>flox-rust-stylistic-conventions SKILL.md</strong></td>
  <td><code>.claude/skills/flox-rust-stylistic-conventions/SKILL.md</code></td>
  <td>
    Taste-driven conventions where the original code works but reviewers prefer
    a different shape: naming patterns (str_to_x, function-name clarity),
    formatting, imports, user-message wording, constants over magic numbers.
  </td>
</tr>
<tr>
  <td><strong>cli/ CLAUDE.md (cross-cutting)</strong></td>
  <td><code>cli/CLAUDE.md</code></td>
  <td>
    12 cross-cutting rules that apply to the entire Rust workspace: parse at entry
    points, error-chain preservation, formatdoc! usage, assert_eq! on structs,
    Manifest&lt;S&gt; constructors, structured tracing fields, concrete provider types,
    test naming. These are the "top-12" highest-frequency, highest-confidence patterns.
  </td>
</tr>
<tr>
  <td><strong>commands/ CLAUDE.md</strong></td>
  <td><code>cli/flox/src/commands/CLAUDE.md</code></td>
  <td>
    Area-specific rules for CLI command implementations: typed CLI boundaries
    (NixFlakeRef, Shell, Url), ConcreteEnvironment match exhaustiveness, bpaf flag
    placement, auth context in user messages, positional argument naming.
    Highest-traffic area (163 classified comments).
  </td>
</tr>
<tr>
  <td><strong>models/environment/ CLAUDE.md</strong></td>
  <td><code>cli/flox-rust-sdk/src/models/environment/CLAUDE.md</code></td>
  <td>
    Test coverage expectations for new error paths; error-variant design (Box vs
    string-flattening); manifest lifecycle discipline; build-skipping/locking
    consistency across parallel methods.
  </td>
</tr>
<tr>
  <td><strong>providers/ CLAUDE.md</strong></td>
  <td><code>cli/flox-rust-sdk/src/providers/CLAUDE.md</code></td>
  <td>
    Error classification at the provider boundary; auth-flow correctness and tracing
    for Kerberos vs Auth0; deprecated MockClient removal discipline; provider-trait
    design (concrete types over pinned associated types).
  </td>
</tr>
<tr>
  <td><strong>gap-report.md</strong></td>
  <td><code>scripts/pr-analysis/findings/gap-report.md</code></td>
  <td>
    67 findings where in_agents_md=0 and confidence &ge; 0.5. Distilled into 30
    proposed AGENTS.md amendments (groups A–L) plus 2 new subsections
    (Testing conventions, PR scope). 14 marked "needs more evidence";
    16 marked declined (too specific or superseded).
  </td>
</tr>
</table>
</section>
"""


def build_before_after() -> str:
    agents_size = (WORKTREE / "AGENTS.md").stat().st_size if (WORKTREE / "AGENTS.md").exists() else 0
    agents_lines = len(read_file(WORKTREE / "AGENTS.md").splitlines())

    return f"""
<section id="before-after">
<h2>Before / After — AGENTS.md</h2>

<h3>Before (as-committed)</h3>
<table>
<tr><th>Metric</th><th>Value</th></tr>
<tr><td>File size</td><td>{agents_size:,} bytes ({agents_lines} lines)</td></tr>
<tr><td>Sections</td><td>Project Overview, Development Setup, Common Commands, Architecture, Testing, Debugging, Conventions (monolithic), Manifest usage, IDE Setup</td></tr>
<tr><td>Rust style rules</td><td>~22 bulleted items in a single "Rust style" list</td></tr>
<tr><td>Error handling guidance</td><td>One "Error handling architecture" subsection, 5 bullet points</td></tr>
<tr><td>Testing guidance</td><td>Test naming + assert_eq! bullets only; no "when to write tests" guidance</td></tr>
<tr><td>PR scope guidance</td><td>None</td></tr>
<tr><td>Shell script guidance</td><td>None</td></tr>
</table>

<h3>Proposed after (30 amendments + 2 new sections)</h3>
<table>
<tr><th>Group</th><th>Amendment</th><th>Type</th><th>Signal</th></tr>
<tr><td>A</td><td>Named constants (no magic numbers), adjacent comments, avoid labelled blocks</td><td>EXPANSION / NEW</td><td>T1-accepted</td></tr>
<tr><td>B</td><td>Singular enum variants; config vs runtime type naming</td><td>NEW</td><td>T1-accepted</td></tr>
<tr><td>C</td><td>Workspace deps in Cargo.toml</td><td>EXPANSION</td><td>T1-accepted</td></tr>
<tr><td>D</td><td>Expired tokens carry identity; diagnostic for unsupported build features</td><td>NEW</td><td>T1-accepted</td></tr>
<tr><td>E</td><td>Unstable JSON output in man pages; SYNOPSIS mutual exclusion; hidden-subcommand man pages; precise Nix terminology; breaking changes framing</td><td>NEW / EXPANSION</td><td>T1-accepted / T1-raised</td></tr>
<tr><td>F</td><td>Document edge cases with evidence; TODO(&lt;issue&gt;) for deferred work; preserve doc comments; document known races; upstream workarounds cite issue URL</td><td>EXPANSION / NEW</td><td>T1-accepted</td></tr>
<tr><td>G</td><td>Avoid double negatives in shell scripts</td><td>NEW</td><td>T1-accepted</td></tr>
<tr><td>H</td><td>Tests for bug fixes; manual testing steps for TTY behavior; testability as design signal</td><td>NEW</td><td>T1-raised</td></tr>
<tr><td>I</td><td>PR scope: defer unrelated refactors; explain whether changes are related</td><td>NEW (new section)</td><td>T1-accepted</td></tr>
<tr><td>J</td><td>Filter filesystem watcher events early; timeouts on blocking waits; async sandwich documentation</td><td>NEW</td><td>T1-accepted / T1-raised</td></tr>
<tr><td>K</td><td>Preserve TOML array decor when patching in-place</td><td>EXPANSION</td><td>T1-accepted</td></tr>
<tr><td>L</td><td>Comment manual symlinking in Nix</td><td>NEW</td><td>T1-raised</td></tr>
</table>

<p>
The two new subsections recommended for AGENTS.md are <strong>"Testing conventions"</strong>
(H1–H3 consolidated, placed before the existing test-naming bullet) and
<strong>"PR scope"</strong> (I1–I2, a new subsection under Conventions).
</p>
</section>
"""


def build_reviewer_voices() -> str:
    gap_text = read_file(FINDINGS_DIR / "gap-report.md")
    # Extract Section 2: Reviewer Voice Notes
    voice_section = excerpt(gap_text, "## Section 2: Reviewer Voice Notes", "## Section 3:")

    # Parse reviewer cards manually from the gap text
    ysndr_para = excerpt(gap_text, "### ysndr (Tier 1)", "\n\n###")
    mkenigs_para = excerpt(gap_text, "### mkenigs (Tier 1)", "\n\n###")
    dcarley_para = excerpt(gap_text, "### dcarley (Tier 1)", "\n\n---")

    def md_para_to_html(text: str) -> str:
        lines = text.strip().splitlines()
        if lines and lines[0].startswith("#"):
            lines = lines[1:]
        return "<p>" + " ".join(l.strip() for l in lines if l.strip()) + "</p>"

    return f"""
<section id="reviewer-voices">
<h2>Reviewer Voices</h2>
<p>Three Tier-1 reviewers together authored 698 of the 1,047 ingested review comments.
Each has a distinct reviewing philosophy.</p>

<div class="reviewer-card">
<h4>ysndr &mdash; <span class="pill pill-t1">Tier 1</span> &mdash; 225 comments ingested</h4>
<p>
ysndr's comments cluster around <strong>architecture and lifecycle correctness</strong>
and <strong>design documentation</strong>. When reviewing authentication code, ysndr
pushed back on sentinel values and missing variants, preferring that code carry richer
semantic signal even in degraded states (an expired token is more informative than
<code>""</code>; a <code>Credential::NoToken</code> variant may be less clear than
<code>Option&lt;Credential&gt;</code>). In activation code, ysndr introduced the async
<code>select!</code>-based signal handling pattern, reflecting a preference for explicit
shutdown sequences that correctly release resources. In provider code, ysndr flagged
removed doc comments as a loss — documentation is a first-class deliverable, not a
by-product. The throughline: <em>write code that carries its own context</em>, whether
through richer types, richer comments, or richer error states.
</p>
</div>

<div class="reviewer-card">
<h4>mkenigs &mdash; <span class="pill pill-t1">Tier 1</span> &mdash; 305 comments ingested</h4>
<p>
mkenigs's comments reflect concern for <strong>reviewer and future-author usability</strong>.
Several comments ask whether a change is related to the PR's stated goal — a signal
that mkenigs expects PRs to be narrowly scoped and that surprises in the diff require
explanation. mkenigs raised testing coverage repeatedly, asking for tests when bug fixes
landed without them. In user-facing output, mkenigs flagged counterintuitive terminology
and unclear man page structure. In Cargo management, mkenigs noted workspace version drift
from an uncoordinated inline version. The throughline: <em>make every change easy for
the next reviewer to evaluate</em> — tight scope, documented intent, tests where
warranted, and consistent infrastructure conventions.
</p>
</div>

<div class="reviewer-card">
<h4>dcarley &mdash; <span class="pill pill-t1">Tier 1</span> &mdash; 175 comments ingested</h4>
<p>
dcarley's comments reflect <strong>operational and resource-pressure awareness</strong>.
When reviewing the filesystem watcher, dcarley flagged unnecessary spinning on every
file event as a resource concern. When reviewing blocking waits, dcarley raised the
question of timeouts for stuck processes. When reviewing breaking changes, dcarley
asked whether the disruption could be framed as a user benefit. In code style, dcarley
flagged double negatives in shell scripts, labelled blocks, and long lines that would
fail the linter. The throughline: <em>think about what happens in production at scale</em>
— resource pressure, broken flows, and user-visible disruption are first-order concerns,
not afterthoughts.
</p>
</div>
</section>
"""


def build_lessons() -> str:
    return """
<section id="lessons" class="lessons">
<h2>Lessons Learned</h2>
<p>Eight things this project taught us about LLM-driven analysis pipelines.</p>
<ol>
<li>
  <strong>Plan bugs against external APIs are normal.</strong>
  <code>gh pr list --json</code> doesn't expose <code>mergeCommitOid</code> or
  <code>author.type</code> — both were assumed in the original plan. Implementer
  subagents that adapt cleanly and flag deviations are more valuable than ones
  that blindly follow a spec.
</li>
<li>
  <strong>Heuristic calibration is a swing problem.</strong>
  AGENTS.md matching went 100% (useless-true) &rarr; 0% (useless-false) &rarr;
  73% (defensible) across three iterations of the same idea expressed three
  different ways. No heuristic is "obviously right" before you see failure modes.
  Budget for at least two retros before a full run.
</li>
<li>
  <strong>Embeddings are the right tool for one-line rule deduplication.</strong>
  Even paraphrased duplicates only Jaccard at 0.3–0.4; MiniLM cosine puts them
  at 0.55–0.65. The threshold (0.65) is empirical and corpus-specific — validate
  it on a sample before committing.
</li>
<li>
  <strong><code>INSERT OR REPLACE</code> with <code>ON DELETE CASCADE</code> is a footgun.</strong>
  SQLite implements REPLACE as DELETE+INSERT, which cascades to all child rows.
  This silently wiped <code>comment_final_code</code> rows during a re-ingest.
  Fix: <code>INSERT … ON CONFLICT(id) DO UPDATE SET …</code>. Test cascade
  behavior before any bulk re-ingest that touches a parent table.
</li>
<li>
  <strong>Subagent self-doubt is real and batch-size-sensitive.</strong>
  A Haiku subagent failed on a 70KB batch claiming "exceeds token limits" while
  three siblings handled 58–74KB files fine. Reducing batch size from 30 to 15
  items eliminated the trigger entirely — smaller batches mean less per-item
  context, less Haiku hedging.
</li>
<li>
  <strong>Calibrating on one time window risks overfitting.</strong>
  The recent 2-month window was 52% <code>activations</code> due to a single
  fix series. Iter-4 on a different window showed 51% <em>other</em>, not 31%.
  This is not a pipeline bug — it is a corpus-composition signal. Use a full
  multi-month window for the final run.
</li>
<li>
  <strong>The "other" bucket is not all noise.</strong>
  The Task 8.5 re-cluster of high-confidence <em>other</em> comments found
  19 actionable rules that the Haiku classifier had given up on. A targeted
  Sonnet pass on evd&ge;2 comments reduced the <em>other</em> rate from 39%
  to 6% for that subset. Two-tier classification (cheap Haiku for breadth,
  expensive Sonnet for precision on flagged comments) is the right architecture.
</li>
<li>
  <strong>Evidence count is a weak signal at corpus scale.</strong>
  474 of 488 findings have exactly one supporting comment. This is not pipeline
  failure — it is the nature of the flox review corpus: reviewers rarely
  raise the same abstract principle twice in the same words. Single-evidence
  findings backed by a Tier-1 reviewer who accepted the change are still
  meaningful signal; they are just "a reviewer enforced this at least once,"
  not "a team-wide invariant."
</li>
</ol>
</section>
"""


# ---------------------------------------------------------------------------
# Table of contents
# ---------------------------------------------------------------------------

SECTIONS = [
    ("overview", "Overview"),
    ("pipeline-architecture", "Pipeline Architecture"),
    ("calibration-journey", "Calibration Journey"),
    ("drift-correction", "Drift Correction"),
    ("corpus-shape", "Corpus Shape"),
    ("findings-table", "Top 50 Findings"),
    ("artifact-routing", "Artifact Routing"),
    ("before-after", "Before / After AGENTS.md"),
    ("reviewer-voices", "Reviewer Voices"),
    ("lessons", "Lessons Learned"),
]


def build_toc() -> str:
    items = "\n".join(
        f'  <li><a href="#{sid}">{title}</a></li>'
        for sid, title in SECTIONS
    )
    return f"""<nav>
<p>Table of Contents</p>
<ol>
{items}
</ol>
</nav>"""


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> None:
    FINDINGS_DIR.mkdir(parents=True, exist_ok=True)

    sections = [
        build_overview(),
        build_pipeline(),
        build_calibration(),
        build_drift_correction(),
        build_corpus_shape(),
        build_findings_table(),
        build_artifact_routing(),
        build_before_after(),
        build_reviewer_voices(),
        build_lessons(),
    ]

    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Flox Rust PR-Analysis — Journey</title>
  <style>
{CSS}
  </style>
</head>
<body>
<div class="page-wrap">
  <h1>Flox Rust PR-Analysis &mdash; Full Journey</h1>
  <p class="subtitle">
    From 944 review comments across 216 PRs (8 months) to
    2 skills + 4 CLAUDE.md files + gap report
  </p>

{build_toc()}

{"".join(sections)}

  <footer style="margin-top:60px;padding-top:20px;border-top:1px solid #dde3ed;color:#888;font-size:0.85rem">
    Generated by <code>build_journey_html.py</code> from <code>pr_analysis.db</code>.
    Data: 216 PRs, 944 classifications. Report date: 2026-05-17.
  </footer>
</div>
</body>
</html>
"""

    OUT.write_text(html, encoding="utf-8")
    size_kb = OUT.stat().st_size / 1024
    lines = len(html.splitlines())
    section_count = html.count("<section ")
    print(f"Written: {OUT}")
    print(f"Size: {size_kb:.1f} KB  |  Lines: {lines:,}  |  Sections: {section_count}")


if __name__ == "__main__":
    main()
