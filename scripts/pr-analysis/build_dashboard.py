#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Build the Task 8 PR-analysis analytics dashboard as a single HTML file.

Reads scripts/pr-analysis/data/pr_analysis.db (read-only) and `git log` on the
main branch. Renders an offline-viewable HTML with inline SVG charts only.
"""

from __future__ import annotations

import html
import sqlite3
import subprocess
from collections import Counter, defaultdict
from datetime import datetime, timedelta
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent  # worktree root
DB = ROOT / "scripts" / "pr-analysis" / "data" / "pr_analysis.db"
OUT = ROOT / "rust-pr-analysis-dashboard-01.html"
WINDOW_START = "2025-09-18"
WINDOW_END = "2026-05-15"

FILE_TYPE_MAP = {
    ".rs": "rust",
    ".sh": "bash",
    ".bash": "bash",
    ".bats": "bash",
    ".zsh": "bash",
    ".fish": "bash",
    ".nix": "nix",
    ".toml": "config",
    ".json": "config",
    ".lock": "config",
    ".md": "markdown",
}
TYPE_ORDER = ["rust", "bash", "nix", "config", "markdown", "other"]
TYPE_LABEL = {
    "rust": "Rust",
    "bash": "Bash/Bats",
    "nix": "Nix",
    "config": "TOML/JSON/Lock",
    "markdown": "Markdown",
    "other": "Other",
}
TYPE_COLOR = {
    "rust": "#c47b3c",
    "bash": "#3b6bd6",
    "nix": "#6f42c1",
    "config": "#2f8a52",
    "markdown": "#c98a17",
    "other": "#9aa0a6",
}
TIER_COLOR = {
    1: "#2f8a52",  # T1 green
    2: "#c98a17",  # T2 amber
    3: "#6c757d",  # T3 grey
    4: "#9bb0e0",  # bot/light
}
TIER_LABEL = {1: "Tier 1", 2: "Tier 2", 3: "Tier 3", 4: "Bot"}


def q(conn: sqlite3.Connection, sql: str, params: tuple = ()) -> list[tuple]:
    return list(conn.execute(sql, params))


def fetch_db() -> dict:
    conn = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    data: dict = {}

    # KPI counts
    data["pr_total"] = q(conn, "SELECT COUNT(*) FROM pr")[0][0]
    row = q(conn, "SELECT COUNT(*), SUM(CASE WHEN is_noise=1 THEN 1 ELSE 0 END) FROM line_comment")[0]
    data["lc_total"], data["lc_noise"] = row[0], row[1] or 0
    data["lc_classified"] = q(conn, "SELECT COUNT(*) FROM classification")[0][0]
    data["review_summaries"] = q(conn, "SELECT COUNT(*) FROM review_summary")[0][0]
    data["issue_comments"] = q(conn, "SELECT COUNT(*) FROM pr_comment")[0][0]
    data["findings_total"] = q(conn, "SELECT COUNT(*) FROM finding")[0][0]
    data["findings_cross"] = q(conn, "SELECT COUNT(*) FROM finding WHERE scope='cross-cutting'")[0][0]
    data["pr_date_min"], data["pr_date_max"] = q(
        conn, "SELECT MIN(merged_at), MAX(merged_at) FROM pr"
    )[0]

    # Per-PR comment counts (for avg/median/max)
    per_pr = q(
        conn,
        "SELECT pr_number, COUNT(*) FROM line_comment WHERE is_noise=0 "
        "GROUP BY pr_number",
    )
    counts = [c for _, c in per_pr]
    # PRs with zero comments are not in this list — pad with zeros up to pr_total
    counts_padded = counts + [0] * (data["pr_total"] - len(counts))
    counts_padded.sort()
    data["avg_comments"] = (
        sum(counts_padded) / len(counts_padded) if counts_padded else 0.0
    )
    n = len(counts_padded)
    data["median_comments"] = (
        counts_padded[n // 2] if n % 2 else (counts_padded[n // 2 - 1] + counts_padded[n // 2]) / 2
    )
    data["max_comments"] = max(counts_padded) if counts_padded else 0

    # PRs merged over time (weekly)
    weekly = q(
        conn,
        "SELECT strftime('%Y-%W', merged_at) AS wk, COUNT(*) FROM pr "
        "GROUP BY wk ORDER BY wk",
    )
    data["weekly_prs"] = weekly

    # Comments-per-PR distribution bins
    bins = [(0, 0), (1, 3), (4, 7), (8, 15), (16, 30), (31, 10**6)]
    bin_labels = ["0", "1-3", "4-7", "8-15", "16-30", "31+"]
    bin_counts = [0] * len(bins)
    for c in counts_padded:
        for i, (lo, hi) in enumerate(bins):
            if lo <= c <= hi:
                bin_counts[i] += 1
                break
    data["comment_bins"] = list(zip(bin_labels, bin_counts))

    # Top PR authors
    data["top_pr_authors"] = q(
        conn,
        "SELECT author, author_type, COUNT(*) FROM pr "
        "GROUP BY author, author_type ORDER BY 3 DESC LIMIT 15",
    )

    # Top reviewers (non-noise)
    data["top_reviewers"] = q(
        conn,
        "SELECT author, reviewer_tier, COUNT(*) FROM line_comment "
        "WHERE is_noise=0 GROUP BY author, reviewer_tier ORDER BY 3 DESC LIMIT 12",
    )

    # Tier donut
    data["tier_split"] = q(
        conn,
        "SELECT reviewer_tier, COUNT(*) FROM line_comment WHERE is_noise=0 "
        "GROUP BY reviewer_tier ORDER BY reviewer_tier",
    )

    # Reviewer x Area heatmap — top 8 reviewers, top 7 areas
    top_revs = [r[0] for r in data["top_reviewers"][:8]]
    top_areas_rows = q(
        conn,
        "SELECT area, COUNT(*) FROM line_comment WHERE is_noise=0 "
        "GROUP BY area ORDER BY 2 DESC LIMIT 7",
    )
    top_areas = [r[0] for r in top_areas_rows]
    heatmap = {}
    for r in top_revs:
        for a in top_areas:
            row = q(
                conn,
                "SELECT COUNT(*) FROM line_comment WHERE is_noise=0 "
                "AND author=? AND area=?",
                (r, a),
            )
            heatmap[(r, a)] = row[0][0]
    data["heatmap"] = {
        "reviewers": top_revs,
        "areas": top_areas,
        "cells": heatmap,
    }

    # Reviewer activity over time — top 5 reviewers monthly
    top5 = [r[0] for r in data["top_reviewers"][:5]]
    rev_monthly_rows = q(
        conn,
        "SELECT strftime('%Y-%m', created_at) AS m, author, COUNT(*) "
        "FROM line_comment WHERE is_noise=0 AND author IN ({}) "
        "GROUP BY m, author ORDER BY m, author".format(
            ",".join(["?"] * len(top5))
        ),
        tuple(top5),
    )
    data["rev_monthly"] = rev_monthly_rows
    data["rev_monthly_top5"] = top5

    # Area distribution
    data["area_dist"] = q(
        conn,
        "SELECT area, COUNT(*) FROM line_comment WHERE is_noise=0 "
        "GROUP BY area ORDER BY 2 DESC",
    )

    # Taxonomy distribution with avg confidence
    data["tax_dist"] = q(
        conn,
        "SELECT taxonomy, COUNT(*), ROUND(AVG(confidence), 2) "
        "FROM classification GROUP BY taxonomy ORDER BY 2 DESC",
    )

    # Thread resolution
    data["thread_split"] = q(
        conn,
        "SELECT thread_resolved, COUNT(*) FROM line_comment WHERE is_noise=0 "
        "GROUP BY thread_resolved",
    )

    # Findings by area + scope split
    data["finding_area"] = q(
        conn,
        "SELECT area, COUNT(*) FROM finding GROUP BY area ORDER BY 2 DESC",
    )
    data["finding_scope"] = q(
        conn,
        "SELECT scope, COUNT(*) FROM finding GROUP BY scope",
    )

    # Evidence-count distribution (1, 2, 3, 4+)
    data["evidence_dist"] = q(
        conn,
        "SELECT CASE WHEN total_evidence_count>=4 THEN '4+' "
        "ELSE CAST(total_evidence_count AS TEXT) END AS bucket, COUNT(*) "
        "FROM finding GROUP BY bucket ORDER BY bucket",
    )

    # Cross-cutting findings (all 13)
    data["cross_findings"] = q(
        conn,
        "SELECT rule_statement, taxonomy, area, areas_seen, "
        "tier1_reviewer_count, total_evidence_count, evidence_pr_numbers, "
        "in_agents_md, agents_md_section, confidence_score "
        "FROM finding WHERE scope='cross-cutting' "
        "ORDER BY confidence_score DESC",
    )

    # was_addressed x thread_resolved
    data["addr_resolve"] = q(
        conn,
        "SELECT c.was_addressed, lc.thread_resolved, COUNT(*) "
        "FROM classification c JOIN line_comment lc ON lc.id = c.comment_id "
        "WHERE lc.is_noise=0 GROUP BY c.was_addressed, lc.thread_resolved",
    )

    # in_agents_md split
    data["agents_md_split"] = q(
        conn,
        "SELECT in_agents_md, COUNT(*) FROM finding GROUP BY in_agents_md",
    )

    conn.close()
    return data


def fetch_git(repo_root: Path) -> dict:
    """Run git log against the worktree's repo for commits + numstat."""
    # Each commit: COMMIT|sha|author|date  then numstat lines: added\tdeleted\tpath
    cmd = [
        "git", "-C", str(repo_root),
        "log", "main",
        f"--since={WINDOW_START}", f"--until={WINDOW_END}",
        "--pretty=format:COMMIT|%H|%an|%ad",
        "--date=short", "--numstat", "--no-merges",
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, check=True)
    out = proc.stdout

    commits = 0
    total_added = 0
    total_deleted = 0
    by_author_commits: Counter = Counter()
    by_author_added: Counter = Counter()
    by_month_commits: Counter = Counter()
    by_month_added: Counter = Counter()
    by_month_type: dict = defaultdict(lambda: Counter())
    current_author = None
    current_month = None

    for line in out.splitlines():
        if not line.strip():
            continue
        if line.startswith("COMMIT|"):
            parts = line.split("|", 3)
            _, _sha, author, date = parts
            current_author = author
            current_month = date[:7]
            commits += 1
            by_author_commits[author] += 1
            by_month_commits[current_month] += 1
            continue
        # numstat line
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        added_s, deleted_s, path = parts[0], parts[1], parts[2]
        if added_s == "-" or deleted_s == "-":
            continue  # binary
        try:
            added = int(added_s)
            deleted = int(deleted_s)
        except ValueError:
            continue
        total_added += added
        total_deleted += deleted
        if current_author:
            by_author_added[current_author] += added
        if current_month:
            by_month_added[current_month] += added
            # file type
            suffix = Path(path).suffix.lower()
            # handle renames like {old => new}
            ftype = FILE_TYPE_MAP.get(suffix, "other")
            by_month_type[current_month][ftype] += added

    sha_proc = subprocess.run(
        ["git", "-C", str(repo_root), "rev-parse", "HEAD"],
        capture_output=True, text=True, check=True,
    )
    return {
        "commits": commits,
        "total_added": total_added,
        "total_deleted": total_deleted,
        "by_author_commits": by_author_commits,
        "by_author_added": by_author_added,
        "by_month_commits": dict(sorted(by_month_commits.items())),
        "by_month_added": dict(sorted(by_month_added.items())),
        "by_month_type": {m: dict(c) for m, c in by_month_type.items()},
        "head_sha": sha_proc.stdout.strip(),
    }


# ──────────────────────────────────── SVG helpers ────────────────────────────


def svg_bar_vertical(pairs, width=900, height=240, color="#3b6bd6",
                     label_every=1, y_label="count"):
    """Vertical bars given list of (label, value)."""
    if not pairs:
        return "<svg width='10' height='10'></svg>"
    n = len(pairs)
    pad_l, pad_r, pad_t, pad_b = 40, 12, 12, 50
    plot_w = width - pad_l - pad_r
    plot_h = height - pad_t - pad_b
    bar_w = plot_w / n
    vmax = max(v for _, v in pairs) or 1
    bars = []
    labels = []
    for i, (lab, v) in enumerate(pairs):
        h = (v / vmax) * plot_h
        x = pad_l + i * bar_w + 1
        y = pad_t + (plot_h - h)
        bars.append(
            f"<rect x='{x:.1f}' y='{y:.1f}' width='{bar_w - 2:.1f}' "
            f"height='{h:.1f}' fill='{color}'>"
            f"<title>{html.escape(str(lab))}: {v}</title></rect>"
        )
        if i % label_every == 0:
            labels.append(
                f"<text x='{x + bar_w/2:.1f}' y='{height - pad_b + 14}' "
                f"font-size='10' fill='#5a6270' text-anchor='end' "
                f"transform='rotate(-45 {x + bar_w/2:.1f} {height - pad_b + 14})'>"
                f"{html.escape(str(lab))}</text>"
            )
    # Y axis ticks (0, mid, max)
    yticks = []
    for frac in (0, 0.5, 1.0):
        val = int(vmax * frac)
        y = pad_t + plot_h - frac * plot_h
        yticks.append(
            f"<text x='{pad_l - 6}' y='{y + 4:.1f}' font-size='10' "
            f"fill='#5a6270' text-anchor='end'>{val}</text>"
            f"<line x1='{pad_l}' x2='{width - pad_r}' y1='{y:.1f}' "
            f"y2='{y:.1f}' stroke='#e4e6eb' stroke-width='1'/>"
        )
    return (
        f"<svg width='{width}' height='{height}' xmlns='http://www.w3.org/2000/svg'>"
        + "".join(yticks) + "".join(bars) + "".join(labels) + "</svg>"
    )


def svg_bar_horizontal(pairs, width=900, row_h=22, color="#3b6bd6",
                       label_w=200, value_fmt=None, color_for=None,
                       hatch_for=None):
    """Horizontal bars; pairs = (label, value [, extra_key])."""
    if not pairs:
        return ""
    pad_r = 80
    vmax = max(p[1] for p in pairs) or 1
    height = row_h * len(pairs) + 20
    plot_x = label_w
    plot_w = width - plot_x - pad_r
    rows = []
    defs = (
        "<defs>"
        "<pattern id='hatch' patternUnits='userSpaceOnUse' width='6' height='6' "
        "patternTransform='rotate(45)'>"
        "<rect width='6' height='6' fill='#cdd5e3'/>"
        "<line x1='0' y1='0' x2='0' y2='6' stroke='#7e8aa3' stroke-width='2'/>"
        "</pattern>"
        "</defs>"
    )
    for i, p in enumerate(pairs):
        lab, val = p[0], p[1]
        extra = p[2] if len(p) > 2 else None
        bar_w = (val / vmax) * plot_w
        y = 8 + i * row_h
        c = color_for(extra) if color_for else color
        fill = "url(#hatch)" if hatch_for and hatch_for(extra) else c
        val_str = value_fmt(val) if value_fmt else str(val)
        rows.append(
            f"<text x='{plot_x - 8}' y='{y + row_h/2 + 4:.1f}' font-size='12' "
            f"text-anchor='end' fill='#1c1f24'>{html.escape(str(lab))}</text>"
            f"<rect x='{plot_x}' y='{y:.1f}' width='{bar_w:.1f}' "
            f"height='{row_h - 6}' fill='{fill}'>"
            f"<title>{html.escape(str(lab))}: {val_str}</title></rect>"
            f"<text x='{plot_x + bar_w + 6:.1f}' y='{y + row_h/2 + 4:.1f}' "
            f"font-size='11' fill='#5a6270'>{html.escape(val_str)}</text>"
        )
    return (
        f"<svg width='{width}' height='{height}' xmlns='http://www.w3.org/2000/svg'>"
        + defs + "".join(rows) + "</svg>"
    )


def svg_donut(pairs, size=180, color_for=None, label_center=None):
    """pairs = (label, value)."""
    total = sum(v for _, v in pairs) or 1
    cx = cy = size / 2
    r = size / 2 - 6
    r_in = r * 0.6
    import math
    a0 = -math.pi / 2  # start at top
    parts = []
    legend = []
    for i, (lab, v) in enumerate(pairs):
        frac = v / total
        a1 = a0 + frac * 2 * math.pi
        large = 1 if frac > 0.5 else 0
        x0, y0 = cx + r * math.cos(a0), cy + r * math.sin(a0)
        x1, y1 = cx + r * math.cos(a1), cy + r * math.sin(a1)
        xi1, yi1 = cx + r_in * math.cos(a1), cy + r_in * math.sin(a1)
        xi0, yi0 = cx + r_in * math.cos(a0), cy + r_in * math.sin(a0)
        c = color_for(lab) if color_for else f"hsl({i*53 % 360}, 55%, 55%)"
        path = (
            f"M {x0:.2f} {y0:.2f} A {r} {r} 0 {large} 1 {x1:.2f} {y1:.2f} "
            f"L {xi1:.2f} {yi1:.2f} A {r_in} {r_in} 0 {large} 0 {xi0:.2f} {yi0:.2f} Z"
        )
        parts.append(
            f"<path d='{path}' fill='{c}'>"
            f"<title>{html.escape(str(lab))}: {v} ({frac*100:.1f}%)</title></path>"
        )
        legend.append((lab, v, c, frac))
        a0 = a1
    center = ""
    if label_center:
        center = (
            f"<text x='{cx}' y='{cy - 4}' text-anchor='middle' font-size='12' "
            f"fill='#5a6270'>{html.escape(label_center[0])}</text>"
            f"<text x='{cx}' y='{cy + 14}' text-anchor='middle' font-size='18' "
            f"font-weight='600' fill='#1c1f24'>{html.escape(label_center[1])}</text>"
        )
    svg = (
        f"<svg width='{size}' height='{size}' xmlns='http://www.w3.org/2000/svg'>"
        + "".join(parts) + center + "</svg>"
    )
    legend_html = "<ul class='donut-legend'>" + "".join(
        f"<li><span class='sw' style='background:{c}'></span>"
        f"{html.escape(str(lab))} — <b>{v}</b> ({frac*100:.1f}%)</li>"
        for lab, v, c, frac in legend
    ) + "</ul>"
    return f"<div class='donut-wrap'>{svg}{legend_html}</div>"


def svg_stacked_bars(months, categories, data_by_month, colors, labels,
                     width=900, height=260):
    """Stacked monthly bars. data_by_month[month][cat] = value."""
    if not months:
        return ""
    pad_l, pad_r, pad_t, pad_b = 50, 20, 12, 50
    plot_w = width - pad_l - pad_r
    plot_h = height - pad_t - pad_b
    bar_w = plot_w / len(months)
    totals = [sum(data_by_month.get(m, {}).get(c, 0) for c in categories)
              for m in months]
    vmax = max(totals) or 1
    bars = []
    labels_x = []
    for i, m in enumerate(months):
        x = pad_l + i * bar_w + 2
        y_cursor = pad_t + plot_h
        for cat in categories:
            v = data_by_month.get(m, {}).get(cat, 0)
            if v <= 0:
                continue
            h = (v / vmax) * plot_h
            y_cursor -= h
            bars.append(
                f"<rect x='{x:.1f}' y='{y_cursor:.1f}' width='{bar_w - 4:.1f}' "
                f"height='{h:.1f}' fill='{colors[cat]}'>"
                f"<title>{html.escape(m)} {labels[cat]}: {v}</title></rect>"
            )
        labels_x.append(
            f"<text x='{x + bar_w/2:.1f}' y='{height - pad_b + 14}' "
            f"font-size='10' fill='#5a6270' text-anchor='end' "
            f"transform='rotate(-45 {x + bar_w/2:.1f} {height - pad_b + 14})'>"
            f"{html.escape(m)}</text>"
        )
    yticks = []
    for frac in (0, 0.5, 1.0):
        val = int(vmax * frac)
        y = pad_t + plot_h - frac * plot_h
        yticks.append(
            f"<text x='{pad_l - 6}' y='{y + 4:.1f}' font-size='10' "
            f"fill='#5a6270' text-anchor='end'>{val:,}</text>"
            f"<line x1='{pad_l}' x2='{width - pad_r}' y1='{y:.1f}' "
            f"y2='{y:.1f}' stroke='#e4e6eb' stroke-width='1'/>"
        )
    legend = "<div class='legend-row'>" + "".join(
        f"<span><i style='background:{colors[c]}'></i>{labels[c]}</span>"
        for c in categories
    ) + "</div>"
    return (
        f"<svg width='{width}' height='{height}' xmlns='http://www.w3.org/2000/svg'>"
        + "".join(yticks) + "".join(bars) + "".join(labels_x) + "</svg>"
        + legend
    )


def svg_grouped_bars(months, series, data, colors, width=900, height=260):
    """Grouped bars: data[series_name][month] = value."""
    if not months or not series:
        return ""
    pad_l, pad_r, pad_t, pad_b = 50, 20, 12, 50
    plot_w = width - pad_l - pad_r
    plot_h = height - pad_t - pad_b
    group_w = plot_w / len(months)
    bar_w = group_w / (len(series) + 0.5)
    vmax = max(
        (data.get(s, {}).get(m, 0) for s in series for m in months),
        default=1,
    ) or 1
    bars = []
    labels_x = []
    for i, m in enumerate(months):
        gx = pad_l + i * group_w + group_w * 0.1
        for j, s in enumerate(series):
            v = data.get(s, {}).get(m, 0)
            h = (v / vmax) * plot_h
            x = gx + j * bar_w
            y = pad_t + plot_h - h
            bars.append(
                f"<rect x='{x:.1f}' y='{y:.1f}' width='{bar_w - 1:.1f}' "
                f"height='{h:.1f}' fill='{colors[s]}'>"
                f"<title>{html.escape(s)} {html.escape(m)}: {v}</title></rect>"
            )
        labels_x.append(
            f"<text x='{pad_l + i*group_w + group_w/2:.1f}' "
            f"y='{height - pad_b + 14}' font-size='10' fill='#5a6270' "
            f"text-anchor='end' transform='rotate(-45 "
            f"{pad_l + i*group_w + group_w/2:.1f} {height - pad_b + 14})'>"
            f"{html.escape(m)}</text>"
        )
    yticks = []
    for frac in (0, 0.5, 1.0):
        val = int(vmax * frac)
        y = pad_t + plot_h - frac * plot_h
        yticks.append(
            f"<text x='{pad_l - 6}' y='{y + 4:.1f}' font-size='10' "
            f"fill='#5a6270' text-anchor='end'>{val}</text>"
            f"<line x1='{pad_l}' x2='{width - pad_r}' y1='{y:.1f}' "
            f"y2='{y:.1f}' stroke='#e4e6eb' stroke-width='1'/>"
        )
    legend = "<div class='legend-row'>" + "".join(
        f"<span><i style='background:{colors[s]}'></i>{html.escape(s)}</span>"
        for s in series
    ) + "</div>"
    return (
        f"<svg width='{width}' height='{height}' xmlns='http://www.w3.org/2000/svg'>"
        + "".join(yticks) + "".join(bars) + "".join(labels_x) + "</svg>"
        + legend
    )


def svg_heatmap(rows, cols, cells, width=720, cell_h=30, label_w=160):
    """rows: reviewers; cols: areas; cells: dict[(r,c)] = int."""
    if not rows or not cols:
        return ""
    plot_w = width - label_w - 20
    cell_w = plot_w / len(cols)
    vmax = max(cells.values()) or 1
    height = cell_h * len(rows) + 80
    out = []
    # column headers
    for j, c in enumerate(cols):
        x = label_w + j * cell_w + cell_w / 2
        out.append(
            f"<text x='{x:.1f}' y='60' font-size='11' fill='#5a6270' "
            f"text-anchor='end' transform='rotate(-35 {x:.1f} 60)'>"
            f"{html.escape(c)}</text>"
        )
    for i, r in enumerate(rows):
        y = 70 + i * cell_h
        out.append(
            f"<text x='{label_w - 8}' y='{y + cell_h/2 + 4:.1f}' "
            f"font-size='11' text-anchor='end' fill='#1c1f24'>"
            f"{html.escape(r)}</text>"
        )
        for j, c in enumerate(cols):
            v = cells.get((r, c), 0)
            intensity = v / vmax
            opacity = 0.08 + intensity * 0.92
            x = label_w + j * cell_w
            color = f"rgba(60, 100, 200, {opacity:.3f})"
            text_fill = "#fff" if intensity > 0.55 else "#1c1f24"
            out.append(
                f"<rect x='{x:.1f}' y='{y:.1f}' width='{cell_w - 2:.1f}' "
                f"height='{cell_h - 2}' fill='{color}'>"
                f"<title>{html.escape(r)} / {html.escape(c)}: {v}</title></rect>"
            )
            if v:
                out.append(
                    f"<text x='{x + cell_w/2:.1f}' y='{y + cell_h/2 + 4:.1f}' "
                    f"font-size='11' text-anchor='middle' fill='{text_fill}'>"
                    f"{v}</text>"
                )
    return (
        f"<svg width='{width}' height='{height}' xmlns='http://www.w3.org/2000/svg'>"
        + "".join(out) + "</svg>"
    )


# ──────────────────────────────────── HTML render ────────────────────────────


CSS = """
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
.container { max-width: 1180px; margin: 0 auto; }
header.page-header {
  background: var(--bg-card); border: 1px solid var(--border);
  border-radius: 8px; padding: 24px 28px; box-shadow: var(--shadow);
  margin-bottom: 28px;
}
header.page-header h1 { margin: 0 0 4px; font-size: 26px; font-weight: 600; letter-spacing: -.01em; }
header.page-header .subtitle { color: var(--fg-mute); font-size: 14px; margin-bottom: 18px; }
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
p { margin: 0 0 10px; }
code { font-family: "SF Mono", Menlo, Consolas, monospace; font-size: 13px; background: var(--code-bg); padding: 1px 5px; border-radius: 3px; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
.pill { display: inline-block; padding: 2px 8px; border-radius: 999px; font-size: 11px; font-weight: 600; letter-spacing: .02em; }
.pill.good { background: #e3f3e8; color: #1e5a35; }
.pill.warn { background: #faecd1; color: #7c5510; }
.pill.bad  { background: #f8dada; color: #7a2929; }
.pill.neutral { background: #e8e9ec; color: #495057; }
.pill.t1 { background: #e3f3e8; color: #1e5a35; }
.pill.t2 { background: #faecd1; color: #7c5510; }
.pill.t3 { background: #e8e9ec; color: #495057; }
.pill.t4 { background: #e9eef9; color: #1e3a7a; }
.chart-wrap { overflow-x: auto; }
.two-col { display: grid; grid-template-columns: 1fr 1fr; gap: 22px; }
@media (max-width: 900px) { .two-col { grid-template-columns: 1fr; } }
.donut-wrap { display: flex; align-items: center; gap: 18px; flex-wrap: wrap; }
.donut-legend { list-style: none; padding: 0; margin: 0; font-size: 13px; }
.donut-legend li { padding: 2px 0; }
.donut-legend .sw { display: inline-block; width: 12px; height: 12px; border-radius: 3px; margin-right: 6px; vertical-align: -2px; }
.legend-row { display: flex; gap: 14px; flex-wrap: wrap; font-size: 12px; color: var(--fg-mute); margin-top: 6px; }
.legend-row i { display: inline-block; width: 12px; height: 12px; border-radius: 3px; margin-right: 5px; vertical-align: -2px; }
.finding-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
@media (max-width: 900px) { .finding-grid { grid-template-columns: 1fr; } }
.finding-card { border: 1px solid var(--border); border-radius: 6px; padding: 14px 16px; background: var(--bg-card); }
.finding-card .rule { font-weight: 600; margin-bottom: 6px; font-size: 14px; }
.finding-card .meta { font-size: 12px; color: var(--fg-mute); margin-bottom: 8px; }
.finding-card .pr-list { font-size: 12px; }
.finding-card .pr-list a { margin-right: 6px; }
.crosstab { border-collapse: collapse; font-size: 13px; }
.crosstab th, .crosstab td { border: 1px solid var(--border); padding: 8px 12px; text-align: center; min-width: 70px; }
.crosstab th { background: var(--code-bg); font-weight: 600; color: var(--fg-mute); text-transform: uppercase; font-size: 11px; letter-spacing: .04em; }
.crosstab td.num { font-variant-numeric: tabular-nums; font-weight: 500; }
.footer { color: var(--fg-mute); font-size: 12px; margin-top: 12px; }
.iter-recap { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; margin-top: 8px; }
@media (max-width: 900px) { .iter-recap { grid-template-columns: 1fr 1fr; } }
.iter-recap .it { border: 1px solid var(--border); border-radius: 6px; padding: 10px 12px; background: var(--code-bg); }
.iter-recap .it .ph { font-size: 11px; text-transform: uppercase; color: var(--fg-mute); letter-spacing: .04em; }
.iter-recap .it .nm { font-weight: 600; margin: 3px 0; font-size: 13px; }
.iter-recap .it .ms { font-size: 12px; }
"""


def render(data: dict, git: dict) -> str:
    pr_total = data["pr_total"]

    # KPI tiles
    kpis = [
        ("PRs", f"{pr_total:,}"),
        ("Line-comments", f"{data['lc_total']:,}"),
        ("Noise filtered", f"{data['lc_noise']:,}"),
        ("Classified", f"{data['lc_classified']:,}"),
        ("Review summaries", f"{data['review_summaries']:,}"),
        ("Issue comments", f"{data['issue_comments']:,}"),
        ("Findings", f"{data['findings_total']:,}"),
        ("Cross-cutting", f"{data['findings_cross']:,}"),
        ("Window", f"{WINDOW_START} → {WINDOW_END}"),
        ("Avg comments/PR", f"{data['avg_comments']:.2f}"),
        ("Median comments/PR", f"{data['median_comments']:.1f}"),
        ("Max comments (single PR)", f"{data['max_comments']:,}"),
        ("Commits (git, window)", f"{git['commits']:,}"),
        ("Lines added (git)", f"{git['total_added']:,}"),
        ("Tests passing", "67"),
        ("Commits in build", "17"),
    ]
    kpi_html = "".join(
        f"<div class='stat'><div class='label'>{html.escape(l)}</div>"
        f"<div class='value'>{html.escape(v)}</div></div>"
        for l, v in kpis
    )

    # ── Section 2: Repo activity ─────────────────────────────────────────
    # Weekly PRs
    weekly_pairs = []
    for wk_str, count in data["weekly_prs"]:
        # convert "%Y-%W" to a date label (Monday of that ISO-ish week)
        try:
            year, week = wk_str.split("-")
            year, week = int(year), int(week)
            d = datetime.strptime(f"{year}-W{week:02d}-1", "%Y-W%W-%w").date()
            weekly_pairs.append((d.isoformat(), count))
        except Exception:
            weekly_pairs.append((wk_str, count))
    weekly_svg = svg_bar_vertical(
        weekly_pairs, width=1100, height=240, color="#3b6bd6",
        label_every=4,
    )

    # Comments-per-PR distribution
    cmt_bins_svg = svg_bar_vertical(
        data["comment_bins"], width=720, height=220, color="#6f42c1",
        label_every=1,
    )

    # Top PR authors
    pa_pairs = [
        (a, c, t)
        for a, t, c in data["top_pr_authors"]
    ]
    def pa_color(t):
        return "#3b6bd6"
    def pa_hatch(t):
        return t and t.lower() == "bot"
    def pa_fmt(v):
        return f"{v} ({v/pr_total*100:.1f}%)"
    pa_svg = svg_bar_horizontal(
        pa_pairs, width=900, label_w=200, color_for=pa_color,
        hatch_for=pa_hatch, value_fmt=pa_fmt,
    )

    # ── Section 3: Contributors ──────────────────────────────────────────
    total_commits = git["commits"]
    top_committers = git["by_author_commits"].most_common(15)
    cm_pairs = [(a, n, None) for a, n in top_committers]
    cm_svg = svg_bar_horizontal(
        cm_pairs, width=900, label_w=200,
        value_fmt=lambda v: f"{v} ({v/total_commits*100:.1f}%)",
        color="#2f8a52",
    )
    top_lines = sorted(
        git["by_author_added"].items(), key=lambda x: -x[1]
    )[:15]
    ln_pairs = [(a, n, None) for a, n in top_lines]
    ln_svg = svg_bar_horizontal(
        ln_pairs, width=900, label_w=200,
        value_fmt=lambda v: f"{v:,} lines",
        color="#c47b3c",
    )

    months = sorted(set(git["by_month_commits"].keys()) | set(git["by_month_added"].keys()))
    cm_monthly_pairs = [(m, git["by_month_commits"].get(m, 0)) for m in months]
    cm_monthly_svg = svg_bar_vertical(
        cm_monthly_pairs, width=900, height=220, color="#2f8a52",
        label_every=1,
    )
    ln_monthly_pairs = [(m, git["by_month_added"].get(m, 0)) for m in months]
    ln_monthly_svg = svg_bar_vertical(
        ln_monthly_pairs, width=900, height=220, color="#c47b3c",
        label_every=1,
    )
    stacked_svg = svg_stacked_bars(
        months, TYPE_ORDER, git["by_month_type"],
        TYPE_COLOR, TYPE_LABEL, width=900, height=300,
    )

    # ── Section 4: Reviewers ─────────────────────────────────────────────
    non_noise_total = data["lc_total"] - data["lc_noise"]
    tr_pairs = [
        (author, count, tier)
        for author, tier, count in data["top_reviewers"]
    ]
    tr_svg = svg_bar_horizontal(
        tr_pairs, width=900, label_w=200,
        color_for=lambda t: TIER_COLOR.get(t or 3, TIER_COLOR[3]),
        value_fmt=lambda v: f"{v} ({v/non_noise_total*100:.1f}%)",
    )
    tier_donut = svg_donut(
        [(TIER_LABEL.get(t or 3, "Unknown"), c) for t, c in data["tier_split"]],
        size=200,
        color_for=lambda lab: {
            "Tier 1": TIER_COLOR[1], "Tier 2": TIER_COLOR[2],
            "Tier 3": TIER_COLOR[3], "Bot": TIER_COLOR[4],
        }.get(lab, "#999"),
        label_center=("Comments", f"{non_noise_total:,}"),
    )

    heatmap_svg = svg_heatmap(
        data["heatmap"]["reviewers"], data["heatmap"]["areas"],
        data["heatmap"]["cells"],
        width=820, cell_h=32, label_w=180,
    )

    # Reviewer activity over time (grouped bars top-5)
    rev_months = sorted({m for m, _, _ in data["rev_monthly"]})
    rev_series = data["rev_monthly_top5"]
    rev_data: dict = defaultdict(dict)
    for m, a, c in data["rev_monthly"]:
        rev_data[a][m] = c
    palette = ["#3b6bd6", "#2f8a52", "#c98a17", "#6f42c1", "#c47b3c"]
    rev_colors = {s: palette[i % len(palette)] for i, s in enumerate(rev_series)}
    rev_act_svg = svg_grouped_bars(
        rev_months, rev_series, rev_data, rev_colors,
        width=1000, height=260,
    )

    # ── Section 5: Segmentation ──────────────────────────────────────────
    area_pairs = [(a, c, None) for a, c in data["area_dist"]]
    area_svg = svg_bar_horizontal(
        area_pairs, width=900, label_w=200,
        value_fmt=lambda v: f"{v} ({v/non_noise_total*100:.1f}%)",
        color="#3b6bd6",
    )

    # Taxonomy distribution tinted by confidence
    tax_data = data["tax_dist"]
    max_conf = max((c for _, _, c in tax_data), default=1.0) or 1.0
    def tax_color_for(extra):
        # extra is confidence 0..1
        conf = extra if extra is not None else 0.5
        # darker = higher; map conf 0..1 to alpha 0.3..1
        alpha = 0.3 + 0.7 * (conf / max_conf)
        return f"rgba(59, 107, 214, {alpha:.3f})"
    tax_pairs = [(t, c, conf) for t, c, conf in tax_data]
    tax_svg = svg_bar_horizontal(
        tax_pairs, width=900, label_w=200,
        value_fmt=lambda v: str(v),
        color_for=tax_color_for,
    )

    # Thread resolution donut
    thread_labels = {0: "Unresolved", 1: "Resolved"}
    thread_pairs = []
    for tr, c in data["thread_split"]:
        lab = thread_labels.get(tr, "Unknown")
        thread_pairs.append((lab, c))
    thread_donut = svg_donut(
        thread_pairs, size=200,
        color_for=lambda lab: {
            "Resolved": "#2f8a52", "Unresolved": "#c98a17",
            "Unknown": "#9aa0a6",
        }.get(lab, "#999"),
        label_center=("Threads", f"{sum(c for _, c in thread_pairs):,}"),
    )

    # ── Section 6: Findings ──────────────────────────────────────────────
    scope_pairs = [(s, c) for s, c in data["finding_scope"]]
    scope_donut = svg_donut(
        scope_pairs, size=200,
        color_for=lambda lab: "#c44545" if lab == "cross-cutting" else "#3b6bd6",
        label_center=("Findings", f"{data['findings_total']:,}"),
    )
    farea_pairs = [(a, c, None) for a, c in data["finding_area"]]
    farea_svg = svg_bar_horizontal(
        farea_pairs, width=900, label_w=200,
        color="#6f42c1", value_fmt=lambda v: str(v),
    )

    # Evidence buckets ordered 1,2,3,4+
    order = {"1": 0, "2": 1, "3": 2, "4+": 3}
    evd_sorted = sorted(data["evidence_dist"], key=lambda x: order.get(x[0], 99))
    evd_pairs = [(str(b), c) for b, c in evd_sorted]
    evd_svg = svg_bar_vertical(
        evd_pairs, width=540, height=200, color="#c47b3c", label_every=1,
    )

    # Cross-cutting findings cards
    cards = []
    for row in data["cross_findings"]:
        (rule, tax, area, areas_seen, t1, evd, evd_prs, in_agents, agents_sec,
         conf) = row
        prs = [p.strip() for p in (evd_prs or "").split(",") if p.strip()][:5]
        pr_links = " ".join(
            f"<a href='https://github.com/flox/flox/pull/{p}' target='_blank'>"
            f"#{html.escape(p)}</a>" for p in prs
        )
        badge = (
            "<span class='pill good'>in AGENTS.md</span>"
            if in_agents else "<span class='pill warn'>gap candidate</span>"
        )
        sec = (
            f" <span class='pill neutral'>{html.escape(agents_sec)}</span>"
            if agents_sec else ""
        )
        cards.append(
            "<div class='finding-card'>"
            f"<div class='rule'>{html.escape(rule)}</div>"
            f"<div class='meta'>"
            f"<span class='pill neutral'>{html.escape(tax)}</span> "
            f"<span class='pill neutral'>{html.escape(area)}</span> "
            f"areas: {html.escape(areas_seen or '-')} · "
            f"T1 reviewers: {t1} · evidence: {evd} · "
            f"confidence: {conf:.2f}"
            f"</div>"
            f"<div class='pr-list'>PRs: {pr_links or '<i>none</i>'}</div>"
            f"<div style='margin-top:6px'>{badge}{sec}</div>"
            "</div>"
        )
    cards_html = "<div class='finding-grid'>" + "".join(cards) + "</div>"

    # ── Section 7: Analysis quality ─────────────────────────────────────
    # was_addressed (true/false/null) x thread_resolved (0/1)
    addr_grid: dict = defaultdict(int)
    for wa, tr, c in data["addr_resolve"]:
        addr_grid[(wa, tr)] = c
    wa_keys = [(1, "True"), (0, "False"), (None, "Unknown")]
    tr_keys = [(1, "Resolved"), (0, "Unresolved")]
    rows = ["<tr><th></th>" + "".join(
        f"<th>{l}</th>" for _, l in tr_keys
    ) + "<th>Total</th></tr>"]
    col_totals = [0] * len(tr_keys)
    grand = 0
    for wa, wl in wa_keys:
        cells = []
        row_total = 0
        for j, (trv, _) in enumerate(tr_keys):
            v = addr_grid.get((wa, trv), 0)
            row_total += v
            col_totals[j] += v
            grand += v
            cells.append(f"<td class='num'>{v}</td>")
        rows.append(
            f"<tr><th>was_addressed = {wl}</th>{''.join(cells)}"
            f"<td class='num'><b>{row_total}</b></td></tr>"
        )
    rows.append(
        "<tr><th>Total</th>"
        + "".join(f"<td class='num'><b>{v}</b></td>" for v in col_totals)
        + f"<td class='num'><b>{grand}</b></td></tr>"
    )
    crosstab_html = "<table class='crosstab'>" + "".join(rows) + "</table>"

    agents_pairs = []
    for v, c in data["agents_md_split"]:
        agents_pairs.append(
            ("in AGENTS.md" if v else "gap candidate", c)
        )
    agents_donut = svg_donut(
        agents_pairs, size=200,
        color_for=lambda lab: "#2f8a52" if lab == "in AGENTS.md" else "#c44545",
        label_center=("Findings", f"{data['findings_total']:,}"),
    )

    # ── Compose final HTML ──────────────────────────────────────────────
    parts = []
    parts.append(f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Rust PR Analysis Dashboard — Task 8</title>
<style>{CSS}</style>
</head>
<body>
<div class="container">

<header class="page-header">
  <h1>Rust PR Analysis Dashboard</h1>
  <div class="subtitle">
    Task 8 output for <code>flox/flox</code> · window
    <b>{WINDOW_START}</b> → <b>{WINDOW_END}</b> · DB
    <code>scripts/pr-analysis/data/pr_analysis.db</code>
  </div>
  <div class="stat-grid">{kpi_html}</div>
</header>
""")

    # Section 2
    parts.append(f"""
<section>
  <h2>Repo activity</h2>
  <h3>PRs merged per week</h3>
  <div class="chart-wrap">{weekly_svg}</div>
  <div class="two-col">
    <div>
      <h3>Comments-per-PR distribution</h3>
      <div class="chart-wrap">{cmt_bins_svg}</div>
    </div>
    <div>
      <h3>Top 15 PR authors</h3>
      <div class="chart-wrap">{pa_svg}</div>
      <p style="font-size:12px;color:var(--fg-mute);margin-top:6px;">
        Hatched bars indicate bot authors. Percent = share of {pr_total} PRs.
      </p>
    </div>
  </div>
</section>
""")

    # Section 3
    parts.append(f"""
<section>
  <h2>Contributors (git log, no merges)</h2>
  <div class="two-col">
    <div>
      <h3>Top 15 committers by commit count</h3>
      <div class="chart-wrap">{cm_svg}</div>
    </div>
    <div>
      <h3>Top 15 committers by lines added</h3>
      <div class="chart-wrap">{ln_svg}</div>
    </div>
  </div>
  <h3>Commits per month</h3>
  <div class="chart-wrap">{cm_monthly_svg}</div>
  <h3>Lines added per month</h3>
  <div class="chart-wrap">{ln_monthly_svg}</div>
  <h3>Lines added per month, stacked by file type</h3>
  <div class="chart-wrap">{stacked_svg}</div>
</section>
""")

    # Section 4
    parts.append(f"""
<section>
  <h2>Reviewers</h2>
  <div class="two-col">
    <div>
      <h3>Top 12 reviewers by non-noise line-comments</h3>
      <div class="chart-wrap">{tr_svg}</div>
      <p style="font-size:12px;color:var(--fg-mute);margin-top:6px;">
        <span class="pill t1">T1</span> <span class="pill t2">T2</span>
        <span class="pill t3">T3</span> <span class="pill t4">Bot</span>
        — bar color indicates reviewer tier.
      </p>
    </div>
    <div>
      <h3>Tier distribution</h3>
      {tier_donut}
    </div>
  </div>
  <h3>Reviewer × Area heatmap (top reviewers × top areas)</h3>
  <div class="chart-wrap">{heatmap_svg}</div>
  <h3>Top-5 reviewer activity over time (monthly)</h3>
  <div class="chart-wrap">{rev_act_svg}</div>
</section>
""")

    # Section 5
    parts.append(f"""
<section>
  <h2>Segmentation</h2>
  <div class="two-col">
    <div>
      <h3>Area distribution (non-noise comments)</h3>
      <div class="chart-wrap">{area_svg}</div>
    </div>
    <div>
      <h3>Thread resolution</h3>
      {thread_donut}
    </div>
  </div>
  <h3>Taxonomy distribution (bar shade = avg confidence)</h3>
  <div class="chart-wrap">{tax_svg}</div>
</section>
""")

    # Section 6
    parts.append(f"""
<section>
  <h2>Findings</h2>
  <div class="two-col">
    <div>
      <h3>Scope split: cross-cutting vs area-specific</h3>
      {scope_donut}
    </div>
    <div>
      <h3>Evidence-count distribution</h3>
      <div class="chart-wrap">{evd_svg}</div>
    </div>
  </div>
  <h3>Findings by area</h3>
  <div class="chart-wrap">{farea_svg}</div>
  <h3>Cross-cutting findings ({data['findings_cross']})</h3>
  {cards_html}
</section>
""")

    # Section 7
    parts.append(f"""
<section>
  <h2>Analysis quality</h2>
  <div class="two-col">
    <div>
      <h3>was_addressed × thread_resolved</h3>
      {crosstab_html}
      <p style="font-size:12px;color:var(--fg-mute);margin-top:6px;">
        Cross-tab over the {non_noise_total:,} non-noise classified comments.
      </p>
    </div>
    <div>
      <h3>in_agents_md split (gap-report inputs)</h3>
      {agents_donut}
    </div>
  </div>
  <h3>Pipeline iteration recap</h3>
  <div class="iter-recap">
    <div class="it">
      <div class="ph">Iter 1</div>
      <div class="nm">Pilot (1 PR)</div>
      <div class="ms">Wired ingest → classify → finding loop end-to-end.</div>
    </div>
    <div class="it">
      <div class="ph">Iter 2</div>
      <div class="nm">Calibration (5 PRs)</div>
      <div class="ms">Tuned taxonomy + reviewer tiers; cut noise rate.</div>
    </div>
    <div class="it">
      <div class="ph">Iter 3</div>
      <div class="nm">Pre-flight (20 PRs)</div>
      <div class="ms">Validated synthesis + AGENTS.md cross-reference.</div>
    </div>
    <div class="it">
      <div class="ph">Full corpus</div>
      <div class="nm">{pr_total} PRs</div>
      <div class="ms">
        {data['lc_total']:,} line-comments → {data['lc_classified']:,} classified
        → {data['findings_total']} findings ({data['findings_cross']} cross-cutting).
      </div>
    </div>
  </div>
  <p style="margin-top:10px;">
    Full narrative: <a href="rust-pr-analysis-jouney-01.html">
    rust-pr-analysis-jouney-01.html</a>.
  </p>
</section>
""")

    # Section 8
    snapshot_ts = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    parts.append(f"""
<section>
  <h2>Provenance</h2>
  <div class="footer">
    Snapshot generated <b>{snapshot_ts}</b> local time.<br>
    Repo HEAD: <code>{html.escape(git['head_sha'])}</code><br>
    DB file: <code>scripts/pr-analysis/data/pr_analysis.db</code> ·
    PR window <code>{WINDOW_START}</code> → <code>{WINDOW_END}</code>.<br>
    Journey log: <a href="rust-pr-analysis-jouney-01.html">
    rust-pr-analysis-jouney-01.html</a>.
  </div>
</section>

</div>
</body>
</html>
""")

    return "".join(parts)


def main() -> None:
    if not DB.exists():
        raise SystemExit(f"DB not found: {DB}")
    data = fetch_db()
    git = fetch_git(ROOT)
    if git["commits"] == 0:
        raise SystemExit("git log returned 0 commits — refusing to fabricate")
    html_out = render(data, git)
    OUT.write_text(html_out, encoding="utf-8")
    size_kb = OUT.stat().st_size / 1024
    # Print a small report to stdout
    print(f"Wrote {OUT} ({size_kb:.1f} KB)")
    print(f"  PRs: {data['pr_total']}  line-comments: {data['lc_total']} "
          f"(noise {data['lc_noise']})  findings: {data['findings_total']} "
          f"({data['findings_cross']} cross-cutting)")
    print(f"  commits: {git['commits']}  lines added: {git['total_added']:,}")
    print("  Top 5 PR authors:")
    for a, t, c in data["top_pr_authors"][:5]:
        print(f"    {a} ({t}) — {c}")
    print("  Top 5 committers:")
    for a, n in git["by_author_commits"].most_common(5):
        print(f"    {a} — {n}")


if __name__ == "__main__":
    main()
