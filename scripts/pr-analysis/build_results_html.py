#!/usr/bin/env python3
"""Build the final-results HTML summary for the Flox PR analysis project.

Reads from:
  - scripts/pr-analysis/data/pr_analysis.db
  - AGENTS.md (root)
  - scripts/pr-analysis/findings/gap-report.md
  - .claude/skills/flox-rust-review/SKILL.md
  - .claude/skills/flox-rust-stylistic-conventions/SKILL.md
  - cli/flox/src/commands/CLAUDE.md
  - cli/flox-rust-sdk/src/providers/CLAUDE.md
  - cli/flox-rust-sdk/src/models/environment/CLAUDE.md

Writes:
  - scripts/pr-analysis/findings/results-summary.html
"""

import re
import sqlite3
from pathlib import Path

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
SCRIPT_DIR = Path(__file__).parent
REPO_ROOT = SCRIPT_DIR.parent.parent
DB_PATH = SCRIPT_DIR / "data" / "pr_analysis.db"
OUT_PATH = SCRIPT_DIR / "findings" / "results-summary.html"

AGENTS_MD = REPO_ROOT / "AGENTS.md"
GAP_REPORT = SCRIPT_DIR / "findings" / "gap-report.md"
SKILL_REVIEW = REPO_ROOT / ".claude/skills/flox-rust-review/SKILL.md"
SKILL_STYLE = REPO_ROOT / ".claude/skills/flox-rust-stylistic-conventions/SKILL.md"
CLAUDE_COMMANDS = REPO_ROOT / "cli/flox/src/commands/CLAUDE.md"
CLAUDE_PROVIDERS = REPO_ROOT / "cli/flox-rust-sdk/src/providers/CLAUDE.md"
CLAUDE_MODELS = REPO_ROOT / "cli/flox-rust-sdk/src/models/environment/CLAUDE.md"

# ---------------------------------------------------------------------------
# DB helpers
# ---------------------------------------------------------------------------
def query(sql, params=()):
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    rows = conn.execute(sql, params).fetchall()
    conn.close()
    return rows

def scalar(sql, params=()):
    rows = query(sql, params)
    if rows:
        return rows[0][0]
    return 0

# ---------------------------------------------------------------------------
# Corpus stats
# ---------------------------------------------------------------------------
def get_corpus_stats():
    pr_count = scalar("SELECT COUNT(*) FROM pr")
    # line_comment is the main review comment table (1047 total, ~944 non-noise)
    comment_count = scalar("SELECT COUNT(*) FROM classification")
    finding_count = scalar("SELECT COUNT(*) FROM finding")
    classification_count = scalar("SELECT COUNT(*) FROM classification")

    # post-noise = comments that passed noise filter
    post_noise = scalar("SELECT COUNT(*) FROM line_comment WHERE is_noise=0")

    top_reviewers = query(
        "SELECT login, weight, tier FROM reviewer ORDER BY weight DESC, login LIMIT 6"
    )

    # comment counts by author from line_comment (exclude bots)
    comment_by_author = query(
        "SELECT author, COUNT(*) as cnt FROM line_comment "
        "WHERE author_type='User' GROUP BY author ORDER BY cnt DESC LIMIT 8"
    )

    area_split = query(
        "SELECT area, COUNT(*) as cnt FROM finding GROUP BY area ORDER BY cnt DESC"
    )

    taxonomy_split = query(
        "SELECT taxonomy, COUNT(*) as cnt FROM finding "
        "GROUP BY taxonomy ORDER BY cnt DESC"
    )

    return {
        "pr_count": pr_count,
        "comment_count": comment_count,
        "finding_count": finding_count,
        "post_noise": post_noise,
        "top_reviewers": [dict(r) for r in top_reviewers],
        "comment_by_author": [dict(r) for r in comment_by_author],
        "area_split": [dict(r) for r in area_split],
        "taxonomy_split": [dict(r) for r in taxonomy_split],
    }

# ---------------------------------------------------------------------------
# Deliverable file metadata
# ---------------------------------------------------------------------------
def count_bold_rules(text):
    """Count **bold** lines as rules (heuristic)."""
    return len(re.findall(r'^\s*\*\*[^*]', text, re.MULTILINE))

def extract_bold_rules(text, limit=10):
    """Extract the first `limit` bold one-liners from a markdown file."""
    rules = []
    for m in re.finditer(r'\*\*([^*\n]{5,120})\*\*', text):
        candidate = m.group(1).strip().rstrip('.')
        if candidate and len(candidate) > 8:
            rules.append(candidate)
        if len(rules) >= limit:
            break
    return rules

def get_deliverables():
    files = [
        {
            "id": "skill-review",
            "label": "Skill: flox-rust-review",
            "path": SKILL_REVIEW,
            "rel_path": ".claude/skills/flox-rust-review/SKILL.md",
            "description": "Correctness rules — error handling, type safety, semantic bugs, testing, provider traits, manifest discipline",
        },
        {
            "id": "skill-style",
            "label": "Skill: flox-rust-stylistic-conventions",
            "path": SKILL_STYLE,
            "rel_path": ".claude/skills/flox-rust-stylistic-conventions/SKILL.md",
            "description": "Style rules — naming, formatting, imports, user-message wording",
        },
        {
            "id": "claude-commands",
            "label": "CLAUDE.md: cli/flox/src/commands/",
            "path": CLAUDE_COMMANDS,
            "rel_path": "cli/flox/src/commands/CLAUDE.md",
            "description": "Area rules for CLI command implementations (type safety, error handling, flag placement, output, testing)",
        },
        {
            "id": "claude-providers",
            "label": "CLAUDE.md: cli/flox-rust-sdk/src/providers/",
            "path": CLAUDE_PROVIDERS,
            "rel_path": "cli/flox-rust-sdk/src/providers/CLAUDE.md",
            "description": "Area rules for provider layer (error classification, auth flows, deprecated infra, trait design)",
        },
        {
            "id": "claude-models",
            "label": "CLAUDE.md: cli/flox-rust-sdk/src/models/environment/",
            "path": CLAUDE_MODELS,
            "rel_path": "cli/flox-rust-sdk/src/models/environment/CLAUDE.md",
            "description": "Area rules for environment models (testing, error variants, manifest lifecycle, build/lock consistency)",
        },
        {
            "id": "gap-report",
            "label": "Gap Report",
            "path": GAP_REPORT,
            "rel_path": "scripts/pr-analysis/findings/gap-report.md",
            "description": "30 proposed AGENTS.md amendments with confidence tier and recommendation",
        },
    ]
    for f in files:
        text = f["path"].read_text(encoding="utf-8")
        f["lines"] = text.count("\n") + 1
        f["rule_count"] = count_bold_rules(text)
        f["rules"] = extract_bold_rules(text, limit=8)
        del f["path"]
    return files

# ---------------------------------------------------------------------------
# Amendments from gap-report.md
# ---------------------------------------------------------------------------
def parse_amendments(gap_text):
    """Parse the summary table from gap-report.md Section 4."""
    amendments = []
    in_table = False
    for line in gap_text.splitlines():
        if line.startswith("| # |"):
            in_table = True
            continue
        if in_table and line.startswith("|---"):
            continue
        if in_table and line.startswith("|"):
            cols = [c.strip() for c in line.strip("|").split("|")]
            if len(cols) >= 7:
                amendments.append({
                    "id": cols[0],
                    "amendment": cols[1],
                    "group": cols[2],
                    "tier": cols[3],
                    "pr": cols[4],
                    "acceptance": cols[5],
                    "recommended": cols[6],
                })
        elif in_table and not line.startswith("|"):
            break
    return amendments

# ---------------------------------------------------------------------------
# AGENTS.md before/after diff
# ---------------------------------------------------------------------------
T1_ACCEPTED_AMENDMENTS = {
    "A1": {
        "after": "Rust style",
        "text": "  - **No magic numbers:** Use named constants rather than bare integer literals.\n    For POSIX file descriptors use `nix::libc::STDERR_FILENO`, `STDOUT_FILENO`,\n    `STDIN_FILENO` rather than `2`, `1`, `0`.",
    },
    "A2": {
        "after": "Use structured log and tracing fields",
        "text": "  - Place comments immediately above the line or block they explain.\n    Don't separate a comment from its target by blank lines or unrelated code.",
    },
    "B1": {
        "after": "Naming new helpers",
        "text": "  - **Enum variants:** Use singular form for variant names (e.g., `Auth0` not `Auth0s`).\n    This is consistent with Rust standard library conventions.",
    },
    "C1": {
        "after": "use guidelines",
        "text": "  - **Cargo dependencies:** Always declare crate dependencies in `Cargo.toml` using\n    `dep.workspace = true` rather than inline version strings.",
    },
    "D1": {
        "after": "Error handling architecture",
        "text": "  - **Expired credentials:** Pass expired tokens to downstream services rather than\n    replacing them with empty strings. An expired token still conveys the identity\n    of the requester, which helps with server-side logging.",
    },
    "D2": {
        "after": "Error handling architecture",
        "text": "  - **Unsupported build features:** When a user-configured mode or feature is not\n    compiled into the current binary, emit a diagnostic message rather than silently\n    falling through. Track missing implementations with a `// TODO(<issue>):` comment.",
    },
    "E3": {
        "after": "User-facing string literals",
        "text": "  - **Hidden subcommands and man pages:** When adding a subcommand behind a feature\n    flag with `#[bpaf(hide)]`, either include the man page immediately or add a\n    `// TODO: add man-pages when we un-hide this` comment.",
    },
    "E4": {
        "after": "Do not surface internal tool output",
        "text": "  - Use precise Nix terminology: 'targets' refers to named build outputs in the\n    manifest; 'artifacts' implies built file paths. Use 'targets' when paths are\n    not yet available.",
    },
    "F1": {
        "after": "Understand semantics before rewriting messages",
        "text": "  - **Documenting edge cases:** When code deliberately handles or skips an edge case\n    because it is rare, document the reasoning inline with a concrete citation so\n    future readers can verify the assumption without reading external sources.",
    },
    "F2": {
        "after": "Deprecated infrastructure",
        "text": "  - **Deferred work:** When deferring an improvement to a follow-up, create a\n    tracking issue and annotate the code with `// TODO(<issue>): description`.\n    A bare `// TODO` without a ticket is harder to prioritize.",
    },
    "F3": {
        "after": "Understand semantics before rewriting messages",
        "text": "  - **Preserving doc comments:** When refactoring or reorganizing code, do not silently\n    remove `///` or `//` comments that explain non-obvious behavior. Either move them\n    to the new home or rewrite them to match the new structure.",
    },
    "F4": {
        "after": "Understand semantics before rewriting messages",
        "text": "  - **Known races and accepted limitations:** When code has a known race condition or\n    constraint accepted without a fix, document it with an inline comment explaining\n    the constraint and why it is acceptable.",
    },
    "F5": {
        "after": "Understand semantics before rewriting messages",
        "text": "  - **Upstream workarounds:** When working around an upstream library bug, cite the\n    upstream issue URL in the doc comment. Prefer filing an upstream PR or issue\n    rather than maintaining the fix locally.",
    },
    "G1": {
        "after": "Bash (activation scripts)",
        "text": "- **Shell script boolean variables:** Prefer positive-assertion variable names over\n  negations (`_run_hook_on_activate` rather than `_no_hook_on_activate`). Avoid\n  double-negation comparisons.",
    },
    "I1": {
        "after": "Commits",
        "text": "- **PR scope:** Keep each PR focused on a single logical change. When encountering\n  an unrelated bug, fix it in a separate PR or add a `// TODO(<issue>):` comment.\n  Note in the PR description whether any changes are opportunistic vs. related.",
    },
    "K1": {
        "after": "Never serialize manifests by hand",
        "text": "- **TOML array in-place editing:** When replacing an element in a `toml_edit::Array`,\n  copy the existing element's `decor` to the replacement value.\n  Use `new_val.decor_mut().clone_from(old_val.decor())`.",
    },
}

def build_before_after_html(agents_text, amendments_map):
    """Build a diff-style view of AGENTS.md with proposed inserts highlighted."""
    lines = agents_text.splitlines()
    # For each T1-accepted amendment, find a line to insert after
    # We'll annotate lines with proposed inserts
    insert_after = {}  # line_index -> list of insert texts

    for aid, info in amendments_map.items():
        anchor = info["after"]
        for i, line in enumerate(lines):
            if anchor.lower() in line.lower():
                idx = insert_after.setdefault(i, [])
                idx.append((aid, info["text"]))
                break  # insert after first match only

    html_lines = []
    for i, line in enumerate(lines):
        escaped = (line
                   .replace("&", "&amp;")
                   .replace("<", "&lt;")
                   .replace(">", "&gt;"))
        html_lines.append(f'<div class="diff-line">{escaped}</div>')
        if i in insert_after:
            for aid, insert_text in insert_after[i]:
                esc_insert = (insert_text
                              .replace("&", "&amp;")
                              .replace("<", "&lt;")
                              .replace(">", "&gt;"))
                insert_lines = esc_insert.split("\n")
                html_lines.append(f'<div class="diff-insert-label">+ [{aid}] proposed insert:</div>')
                for il in insert_lines:
                    html_lines.append(f'<div class="diff-insert">{il}</div>')
    return "\n".join(html_lines)

# ---------------------------------------------------------------------------
# Bar chart helper
# ---------------------------------------------------------------------------
def hbar(label, value, max_value, color="#4a90d9", width=280):
    pct = int(value / max_value * width) if max_value else 0
    return (
        f'<div class="bar-row">'
        f'<span class="bar-label">{label}</span>'
        f'<span class="bar-track">'
        f'<span class="bar-fill" style="width:{pct}px;background:{color}"></span>'
        f'</span>'
        f'<span class="bar-value">{value}</span>'
        f'</div>'
    )

# ---------------------------------------------------------------------------
# HTML render helpers
# ---------------------------------------------------------------------------
def esc(s):
    return str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")

def tier_badge(tier_str):
    tier_str = tier_str.strip()
    if "T1-accepted" in tier_str:
        return '<span class="badge t1-accepted">T1-accepted</span>'
    elif "T1-raised" in tier_str:
        return '<span class="badge t1-raised">T1-raised</span>'
    else:
        return '<span class="badge t2-only">T2-only</span>'

def recommended_badge(rec):
    rec = rec.strip().lower()
    if rec.startswith("yes"):
        return '<span class="badge rec-yes">Yes</span>'
    elif "needs" in rec:
        return '<span class="badge rec-needs">Needs evidence</span>'
    else:
        return '<span class="badge rec-weak">Weak</span>'

# ---------------------------------------------------------------------------
# Main build
# ---------------------------------------------------------------------------
def build():
    corpus = get_corpus_stats()
    deliverables = get_deliverables()
    gap_text = GAP_REPORT.read_text(encoding="utf-8")
    agents_text = AGENTS_MD.read_text(encoding="utf-8")
    amendments = parse_amendments(gap_text)

    # Total line counts
    total_deliverable_lines = sum(d["lines"] for d in deliverables)
    total_deliverable_rules = sum(d["rule_count"] for d in deliverables if d["id"] != "gap-report")

    # Bar chart max values
    area_max = corpus["area_split"][0]["cnt"] if corpus["area_split"] else 1
    tax_max = corpus["taxonomy_split"][0]["cnt"] if corpus["taxonomy_split"] else 1

    # Before/after html
    before_after_html = build_before_after_html(agents_text, T1_ACCEPTED_AMENDMENTS)

    # ---------------------------------------------------------------------------
    # CSS
    # ---------------------------------------------------------------------------
    css = """
      * { box-sizing: border-box; margin: 0; padding: 0; }
      body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
             font-size: 14px; line-height: 1.55; color: #1a1a1a;
             background: #f7f8fa; padding: 24px 16px; }
      .page { max-width: 900px; margin: 0 auto; background: #fff;
              border: 1px solid #e0e0e0; border-radius: 6px; padding: 32px 40px; }
      h1 { font-size: 22px; font-weight: 700; margin-bottom: 4px; }
      .subtitle { color: #555; font-size: 13px; margin-bottom: 28px; }
      nav { background: #f0f4ff; border: 1px solid #d0dbf5; border-radius: 5px;
            padding: 12px 18px; margin-bottom: 32px; }
      nav a { color: #2a5fd4; text-decoration: none; margin-right: 18px;
              font-size: 13px; }
      nav a:hover { text-decoration: underline; }
      section { margin-bottom: 40px; }
      h2 { font-size: 17px; font-weight: 700; border-bottom: 2px solid #2a5fd4;
           padding-bottom: 6px; margin-bottom: 16px; color: #1a2a5a; }
      h3 { font-size: 14px; font-weight: 600; margin: 14px 0 8px; color: #2a3a6a; }
      p { margin-bottom: 10px; }
      table { width: 100%; border-collapse: collapse; font-size: 13px; margin-bottom: 12px; }
      th { background: #f0f4ff; border: 1px solid #d0dbf5; padding: 7px 10px;
           text-align: left; font-weight: 600; }
      td { border: 1px solid #e4e8f0; padding: 6px 10px; vertical-align: top; }
      tr:nth-child(even) td { background: #fafbff; }
      code { background: #f0f0f0; border-radius: 3px; padding: 1px 4px;
             font-family: "SFMono-Regular", Consolas, monospace; font-size: 12px; }
      .badge { display: inline-block; border-radius: 3px; padding: 1px 7px;
               font-size: 11px; font-weight: 600; white-space: nowrap; }
      .t1-accepted { background: #d4f4d4; color: #1a5a1a; }
      .t1-raised   { background: #fff3cd; color: #7a5a00; }
      .t2-only     { background: #f0d0d0; color: #7a1a1a; }
      .rec-yes     { background: #d4f4d4; color: #1a5a1a; }
      .rec-needs   { background: #fff3cd; color: #7a5a00; }
      .rec-weak    { background: #efefef; color: #555; }

      /* Bar charts */
      .bar-row { display: flex; align-items: center; margin-bottom: 5px; }
      .bar-label { width: 180px; font-size: 12px; color: #333; overflow: hidden;
                   text-overflow: ellipsis; white-space: nowrap; flex-shrink: 0; }
      .bar-track { display: inline-block; width: 280px; height: 14px;
                   background: #eee; border-radius: 3px; overflow: hidden;
                   flex-shrink: 0; margin: 0 8px; }
      .bar-fill  { display: block; height: 100%; border-radius: 3px; }
      .bar-value { font-size: 12px; color: #555; }
      .charts-row { display: flex; gap: 40px; flex-wrap: wrap; }
      .chart-block { flex: 1; min-width: 280px; }

      /* Diff / before-after */
      .diff-outer { border: 1px solid #d0d7de; border-radius: 5px; overflow: hidden;
                    font-family: "SFMono-Regular", Consolas, monospace;
                    font-size: 11.5px; max-height: 520px; overflow-y: auto; }
      .diff-line   { padding: 1px 10px; color: #24292f; white-space: pre-wrap;
                     word-break: break-word; }
      .diff-insert { padding: 1px 10px; background: #d4f4d4; color: #1a5a1a;
                     white-space: pre-wrap; word-break: break-word; }
      .diff-insert-label { padding: 2px 10px 0; background: #c8edcc; color: #1a5a1a;
                           font-size: 10.5px; font-weight: 700; }
      .diff-line:hover { background: #f6f8fa; }

      /* Rule lists */
      ul.rules-list { list-style: none; padding: 0; }
      ul.rules-list li { padding: 3px 0; font-size: 12.5px; color: #333; }
      ul.rules-list li::before { content: "→ "; color: #2a5fd4; }

      /* Stats row */
      .stat-row { display: flex; gap: 20px; flex-wrap: wrap; margin-bottom: 16px; }
      .stat-box { background: #f0f4ff; border: 1px solid #d0dbf5; border-radius: 5px;
                  padding: 12px 18px; text-align: center; min-width: 110px; }
      .stat-box .stat-n { font-size: 26px; font-weight: 700; color: #2a5fd4; }
      .stat-box .stat-l { font-size: 11px; color: #555; margin-top: 2px; }

      .note { background: #fffbe6; border-left: 3px solid #f0c000; padding: 8px 12px;
              border-radius: 3px; font-size: 12.5px; color: #5a4a00; margin: 10px 0; }
    """

    # ---------------------------------------------------------------------------
    # Section: Corpus
    # ---------------------------------------------------------------------------
    area_bars = "".join(
        hbar(r["area"], r["cnt"], area_max, "#4a90d9")
        for r in corpus["area_split"]
    )
    tax_bars = "".join(
        hbar(r["taxonomy"], r["cnt"], tax_max, "#7b60d4")
        for r in corpus["taxonomy_split"]
    )

    reviewer_rows = ""
    for rv in corpus["top_reviewers"]:
        tier_label = "Tier 1" if rv["tier"] == 1 else "Tier 2"
        reviewer_rows += f'<tr><td><code>{esc(rv["login"])}</code></td><td>{tier_label}</td><td>{rv["weight"]:.0f}</td></tr>\n'

    section_corpus = f"""
<section id="corpus">
  <h2>Corpus</h2>
  <div class="stat-row">
    <div class="stat-box"><div class="stat-n">{corpus["pr_count"]}</div><div class="stat-l">PRs ingested</div></div>
    <div class="stat-box"><div class="stat-n">{corpus["comment_count"]}</div><div class="stat-l">review comments</div></div>
    <div class="stat-box"><div class="stat-n">{corpus["post_noise"]}</div><div class="stat-l">post-noise</div></div>
    <div class="stat-box"><div class="stat-n">{corpus["finding_count"]}</div><div class="stat-l">findings extracted</div></div>
    <div class="stat-box"><div class="stat-n">5</div><div class="stat-l">deliverables</div></div>
  </div>
  <p>8 months of merged PRs. 1,047 raw line comments ingested; {corpus["post_noise"]} passed the noise filter ({1047 - corpus["post_noise"]} discarded as bot/trivial acks). The {corpus["comment_count"]} classified comments were deduplicated and synthesized into {corpus["finding_count"]} unique findings.</p>

  <h3>Tier reviewers</h3>
  <table style="width:auto;margin-bottom:16px">
    <tr><th>Login</th><th>Tier</th><th>Weight</th></tr>
    {reviewer_rows}
  </table>

  <div class="charts-row">
    <div class="chart-block">
      <h3>Findings by area</h3>
      {area_bars}
    </div>
    <div class="chart-block">
      <h3>Findings by taxonomy</h3>
      {tax_bars}
    </div>
  </div>
</section>
"""

    # ---------------------------------------------------------------------------
    # Section: Deliverables
    # ---------------------------------------------------------------------------
    deliv_rows = ""
    for d in deliverables:
        rc = d["rule_count"] if d["id"] != "gap-report" else "30 amendments"
        deliv_rows += (
            f'<tr><td><a href="#{d["id"]}">{esc(d["label"])}</a></td>'
            f'<td><code>{esc(d["rel_path"])}</code></td>'
            f'<td style="text-align:right">{d["lines"]}</td>'
            f'<td style="text-align:right">{rc}</td>'
            f'<td>{esc(d["description"])}</td></tr>\n'
        )

    section_deliverables = f"""
<section id="deliverables">
  <h2>Deliverables</h2>
  <p>Total across all files: <strong>{total_deliverable_lines} lines</strong>, <strong>{total_deliverable_rules} rules</strong> (skills + area CLAUDE.md files).</p>
  <table>
    <tr><th>Deliverable</th><th>Path</th><th style="text-align:right">Lines</th><th style="text-align:right">Rules</th><th>Description</th></tr>
    {deliv_rows}
  </table>
</section>
"""

    # ---------------------------------------------------------------------------
    # Section: Routing (top rules per deliverable)
    # ---------------------------------------------------------------------------
    routing_html = ""
    for d in deliverables:
        if not d["rules"]:
            continue
        rule_items = "".join(f"<li>{esc(r)}</li>" for r in d["rules"])
        routing_html += f"""
  <div id="{d['id']}" style="margin-bottom:20px">
    <h3>{esc(d['label'])}</h3>
    <p style="font-size:12px;color:#555">{esc(d['rel_path'])}</p>
    <ul class="rules-list">{rule_items}</ul>
  </div>
"""

    section_routing = f"""
<section id="routing">
  <h2>What Goes in Each File</h2>
  <p>Top rules extracted from each deliverable (bold one-liners). See the files directly for full context and evidence citations.</p>
  {routing_html}
</section>
"""

    # ---------------------------------------------------------------------------
    # Section: AGENTS.md before/after
    # ---------------------------------------------------------------------------
    t1_accepted_ids = ", ".join(sorted(T1_ACCEPTED_AMENDMENTS.keys()))

    section_agents = f"""
<section id="agents-md-before-after">
  <h2>AGENTS.md — Proposed Amendments (16 T1-accepted inserts)</h2>
  <p>
    The view below shows the current <code>AGENTS.md</code> with 16 T1-accepted proposed inserts highlighted in green.
    Each insert is identified by its amendment ID (A1, B1, …, K1) from the gap report.
    Inserts are positioned immediately after the anchor text most semantically relevant to each rule.
  </p>
  <div class="note">Amendments shown: {t1_accepted_ids}. Only T1-accepted (acceptance_rate=1.0 + Tier-1 reviewer) inserts are included.
  The remaining 14 amendments require more evidence or are T2-only — see the gap report.</div>
  <div class="diff-outer">{before_after_html}</div>
</section>
"""

    # ---------------------------------------------------------------------------
    # Section: Amendments summary
    # ---------------------------------------------------------------------------
    amend_rows = ""
    for a in amendments:
        amend_rows += (
            f'<tr>'
            f'<td><strong>{esc(a["id"])}</strong></td>'
            f'<td>{esc(a["amendment"])}</td>'
            f'<td>{esc(a["group"])}</td>'
            f'<td>{tier_badge(a["tier"])}</td>'
            f'<td><code>{esc(a["pr"])}</code></td>'
            f'<td>{esc(a["acceptance"])}</td>'
            f'<td>{recommended_badge(a["recommended"])}</td>'
            f'</tr>\n'
        )

    t1_accepted_count = sum(1 for a in amendments if "T1-accepted" in a["tier"])
    t1_raised_count = sum(1 for a in amendments if "T1-raised" in a["tier"])
    t2_only_count = sum(1 for a in amendments if "T2-only" in a["tier"])
    yes_count = sum(1 for a in amendments if a["recommended"].strip().lower().startswith("yes"))

    section_amendments = f"""
<section id="amendments-summary">
  <h2>All 30 Proposed Amendments</h2>
  <p>
    <span class="badge t1-accepted">T1-accepted</span> {t1_accepted_count} — Tier-1 reviewer raised it, author accepted it. Strongest signal.<br>
    <span class="badge t1-raised">T1-raised</span> {t1_raised_count} — Tier-1 reviewer raised it; outcome unknown or not adopted. Needs more evidence.<br>
    <span class="badge t2-only">T2-only</span> {t2_only_count} — Only Tier-2 reviewers raised it. Lowest confidence.<br>
    <strong>{yes_count} amendments recommended</strong> for adoption now (shown in green rows).
  </p>
  <div class="note">
    Critical caveat: every finding has <code>total_evidence_count=1</code>. No rule is backed by more than one independently confirmed comment.
    T1-accepted is the strongest signal available, but it still represents a single data point.
  </div>
  <table>
    <tr>
      <th>ID</th><th>Amendment</th><th>Group</th><th>Tier</th>
      <th>PR</th><th>Accept</th><th>Recommended</th>
    </tr>
    {amend_rows}
  </table>
</section>
"""

    # ---------------------------------------------------------------------------
    # Assemble
    # ---------------------------------------------------------------------------
    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Flox Rust Review Rules — Final Results</title>
  <style>{css}</style>
</head>
<body>
<div class="page">
  <h1>Flox Rust Review Rules — Final Results</h1>
  <p class="subtitle">
    {corpus["pr_count"]} PRs &times; {corpus["comment_count"]} review comments
    &rarr; {corpus["post_noise"]} post-noise
    &rarr; {corpus["finding_count"]} findings
    &rarr; 5 deliverables + gap report
  </p>

  <nav>
    <strong>Sections:</strong>
    <a href="#corpus">Corpus</a>
    <a href="#deliverables">Deliverables</a>
    <a href="#routing">Routing</a>
    <a href="#agents-md-before-after">AGENTS.md diff</a>
    <a href="#amendments-summary">Amendments (30)</a>
  </nav>

  {section_corpus}
  {section_deliverables}
  {section_routing}
  {section_agents}
  {section_amendments}
</div>
</body>
</html>"""

    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(html, encoding="utf-8")
    lines = html.count("\n") + 1
    size_kb = len(html.encode("utf-8")) / 1024
    print(f"Written: {OUT_PATH}")
    print(f"  Lines : {lines}")
    print(f"  Size  : {size_kb:.1f} KB")
    print(f"  Sections: corpus, deliverables, routing, agents-md-before-after, amendments-summary")

if __name__ == "__main__":
    build()
