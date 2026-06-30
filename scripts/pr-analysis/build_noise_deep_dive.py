#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Audit the PR-analysis noise filter and the stylistic-convention classifier.

Reads scripts/pr-analysis/data/pr_analysis.db (read-only) and re-runs the
compiled regexes from scripts.pr_analysis.lib.noise_filter against every
filtered line-comment to produce a forensic per-regex breakdown.

Renders a single self-contained HTML report at the worktree root. Inline SVG
only, no JS, no CDN.

This script AUDITS the noise filter; it does not modify it or the DB.
"""

from __future__ import annotations

import html
import json
import sqlite3
import sys
from collections import Counter, defaultdict
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent  # worktree root
DB = ROOT / "scripts" / "pr-analysis" / "data" / "pr_analysis.db"
OUT = ROOT / "rust-pr-analysis-noise-deep-dive-01.html"

# Make `scripts.pr_analysis.lib.noise_filter` importable. The package directory
# is `scripts/pr-analysis/` (with a hyphen) so we point sys.path at it directly
# and import the bare `lib.noise_filter` module.
LIB_PARENT = ROOT / "scripts" / "pr-analysis"
sys.path.insert(0, str(LIB_PARENT))
from lib import noise_filter as nf  # noqa: E402


TIER_LABEL = {1: "Tier 1", 2: "Tier 2", 3: "Tier 3", 4: "Bot"}
TIER_PILL = {1: "t1", 2: "t2", 3: "t3", 4: "t4"}
STYLISTIC = ["naming", "formatting-style", "imports", "control-flow", "logging-tracing"]


# ──────────────────────────────────────────────────────────────────────────
# Forensic regex match
# ──────────────────────────────────────────────────────────────────────────

def which_regex(body: str) -> str:
    """Mirror the order in is_noise() and return which regex caught the body."""
    if not body or not body.strip():
        return "empty"
    if nf._URL_ONLY.match(body):
        return "url-only"
    if nf._SUGGESTION_ONLY.match(body):
        return "suggestion-only"
    if nf._LGTM_ONLY.match(body):
        return "lgtm-only"
    if nf._PRAISE_NIT_ONLY.match(body):
        return "praise-nit-only"
    # Shouldn't happen for is_noise=1 rows, but guard:
    return "unmatched"


CATEGORY_LABEL = {
    "url-only": "URL-only (bare commit/PR URL)",
    "suggestion-only": "Suggestion-block-only (raw ```suggestion``` blocks)",
    "lgtm-only": "LGTM/thanks/emoji-only",
    "praise-nit-only": "Praise/nit-prefix-only (≤40 char body)",
    "empty": "Empty body (catch-all)",
    "unmatched": "Unmatched (filter drift)",
}
CATEGORY_ORDER = ["url-only", "suggestion-only", "lgtm-only", "praise-nit-only", "empty", "unmatched"]


# ──────────────────────────────────────────────────────────────────────────
# DB queries
# ──────────────────────────────────────────────────────────────────────────

def q(conn, sql, params=()):
    return list(conn.execute(sql, params))


def fetch(conn):
    data = {}

    row = q(conn, "SELECT COUNT(*), SUM(CASE WHEN is_noise=1 THEN 1 ELSE 0 END) FROM line_comment")[0]
    data["lc_total"] = row[0]
    data["lc_noise"] = row[1] or 0
    data["lc_classified"] = q(conn, "SELECT COUNT(*) FROM classification")[0][0]
    data["lc_t4"] = q(conn, "SELECT COUNT(*) FROM line_comment WHERE reviewer_tier=4")[0][0]

    # Average length: noise vs non-noise
    row = q(conn, "SELECT AVG(LENGTH(body)) FROM line_comment WHERE is_noise=1")[0]
    data["avg_len_noise"] = row[0] or 0
    row = q(conn, "SELECT AVG(LENGTH(body)) FROM line_comment WHERE is_noise=0")[0]
    data["avg_len_signal"] = row[0] or 0

    # Filtered set (the protagonists of Section 2)
    data["noise_rows"] = q(
        conn,
        "SELECT id, pr_number, author, reviewer_tier, body "
        "FROM line_comment WHERE is_noise=1 ORDER BY id",
    )

    # Tier breakdown: total and filtered
    data["tier_counts"] = q(
        conn,
        "SELECT reviewer_tier, COUNT(*) AS total, "
        "SUM(CASE WHEN is_noise=1 THEN 1 ELSE 0 END) AS noise "
        "FROM line_comment GROUP BY reviewer_tier ORDER BY reviewer_tier",
    )

    # Substantive samples for the side-by-side
    data["substantive_samples_by_tax"] = {}
    for tax in [
        "naming", "error-handling", "testing", "control-flow",
        "semantic-correctness", "user-facing-messages",
    ]:
        data["substantive_samples_by_tax"][tax] = q(
            conn,
            "SELECT lc.id, lc.author, lc.reviewer_tier, lc.body, "
            "       c.taxonomy, c.rule_statement, c.confidence "
            "FROM line_comment lc "
            "JOIN classification c ON c.comment_id=lc.id "
            "WHERE lc.is_noise=0 AND c.rule_statement IS NOT NULL AND TRIM(c.rule_statement)!='' "
            "AND c.confidence >= 0.6 AND c.taxonomy=? "
            "ORDER BY c.confidence DESC LIMIT 3",
            (tax,),
        )

    # Stylistic taxonomies — full rows
    placeholders = ",".join("?" for _ in STYLISTIC)
    data["stylistic_rows"] = q(
        conn,
        f"SELECT c.comment_id, lc.author, lc.reviewer_tier, lc.area, "
        f"       c.taxonomy, c.rule_statement, c.confidence, "
        f"       c.was_addressed, lc.thread_resolved, lc.body "
        f"FROM classification c "
        f"JOIN line_comment lc ON lc.id=c.comment_id "
        f"WHERE c.taxonomy IN ({placeholders}) "
        f"ORDER BY c.taxonomy, c.confidence DESC",
        tuple(STYLISTIC),
    )

    # Map comment_id -> in_agents_md (via finding.evidence_comment_ids JSON list)
    findings = q(
        conn,
        "SELECT id, in_agents_md, evidence_comment_ids FROM finding",
    )
    cid_to_agents = {}
    for fid, in_md, ev_json in findings:
        try:
            ids = json.loads(ev_json)
        except Exception:
            ids = []
        for cid in ids:
            # If any finding covering this comment is in_agents_md, treat as covered.
            cid_to_agents[cid] = max(cid_to_agents.get(cid, 0), int(in_md))
    data["cid_in_agents"] = cid_to_agents

    # Borderline punts: not noise, taxonomy 'other', confidence < 0.3
    data["borderline_punts"] = q(
        conn,
        "SELECT lc.id, lc.author, lc.reviewer_tier, lc.body, "
        "       c.rule_statement, c.confidence "
        "FROM classification c JOIN line_comment lc ON lc.id=c.comment_id "
        "WHERE lc.is_noise=0 AND c.taxonomy='other' AND c.confidence < 0.3 "
        "ORDER BY c.confidence ASC LIMIT 12",
    )

    # DB mtime
    data["db_mtime"] = datetime.fromtimestamp(DB.stat().st_mtime).isoformat(timespec="seconds")

    return data


# ──────────────────────────────────────────────────────────────────────────
# Helpers
# ──────────────────────────────────────────────────────────────────────────

def esc(s):
    if s is None:
        return ""
    return html.escape(str(s))


def truncate_body(body: str, n: int = 400) -> str:
    body = body or ""
    body_one_line = body.replace("\r\n", "\n")
    if len(body_one_line) > n:
        body_one_line = body_one_line[: n - 1].rstrip() + "…"
    return body_one_line


def tier_pill(tier):
    cls = TIER_PILL.get(tier or 3, "t3")
    label = TIER_LABEL.get(tier or 3, f"T{tier}")
    return f"<span class='pill {cls}'>{esc(label)}</span>"


# ──────────────────────────────────────────────────────────────────────────
# Section renderers
# ──────────────────────────────────────────────────────────────────────────

def render_section1(data, by_cat):
    # Tier table
    tier_rows = []
    for t, total, noise in data["tier_counts"]:
        noise = noise or 0
        rate = (noise / total * 100) if total else 0
        tier_rows.append(
            f"<tr><td>{tier_pill(t)}</td>"
            f"<td class='num'>{total:,}</td>"
            f"<td class='num'>{noise:,}</td>"
            f"<td class='num'>{rate:.1f}%</td></tr>"
        )
    tier_html = (
        "<table class='tbl'><thead><tr>"
        "<th>Tier</th><th>Total</th><th>Filtered as noise</th><th>Noise rate</th>"
        "</tr></thead><tbody>" + "".join(tier_rows) + "</tbody></table>"
    )

    noise_pct = data["lc_noise"] / data["lc_total"] * 100 if data["lc_total"] else 0
    cat_summary_bits = []
    for cat in CATEGORY_ORDER:
        if cat in by_cat and by_cat[cat]:
            n = len(by_cat[cat])
            cat_summary_bits.append(f"{CATEGORY_LABEL[cat].split(' (')[0]}: <strong>{n}</strong>")
    cat_summary = " · ".join(cat_summary_bits)

    return f"""
<section id="s1">
  <h2>1. Headline KPIs</h2>
  <div class="stat-grid">
    <div class="stat"><div class="label">Total line-comments</div><div class="value">{data['lc_total']:,}</div></div>
    <div class="stat"><div class="label">Filtered as noise</div><div class="value">{data['lc_noise']:,} ({noise_pct:.1f}%)</div></div>
    <div class="stat"><div class="label">Classified</div><div class="value">{data['lc_classified']:,}</div></div>
    <div class="stat"><div class="label">Tier-4 (bots)</div><div class="value">{data['lc_t4']:,}</div></div>
    <div class="stat"><div class="label">Avg length — noise</div><div class="value">{data['avg_len_noise']:.0f} chars</div></div>
    <div class="stat"><div class="label">Avg length — signal</div><div class="value">{data['avg_len_signal']:.0f} chars</div></div>
  </div>
  <h3>Forensic regex breakdown (Section 2 preview)</h3>
  <p class="muted">{cat_summary}</p>
  <h3>Noise rate by reviewer tier</h3>
  {tier_html}
</section>
"""


def render_section2(by_cat):
    cards = []
    total_noise = sum(len(v) for v in by_cat.values())
    for cat in CATEGORY_ORDER:
        rows = by_cat.get(cat, [])
        if not rows:
            continue
        pct = (len(rows) / total_noise * 100) if total_noise else 0
        # 6-8 examples
        examples = rows[: 8]
        ex_rows = []
        for r in examples:
            body = truncate_body(r["body"], 280)
            blen = len(r["body"] or "")
            length_flag = ""
            if cat == "praise-nit-only" and 30 <= blen <= 50:
                length_flag = " <span class='pill warn'>borderline len {} </span>".format(blen)
            ex_rows.append(
                f"<tr>"
                f"<td>{tier_pill(r['tier'])} <span class='auth'>{esc(r['author'])}</span></td>"
                f"<td class='body-cell'><code class='body'>{esc(body)}</code>{length_flag}</td>"
                f"</tr>"
            )
        tbl = (
            "<table class='tbl examples'><thead><tr>"
            "<th style='width:160px;'>Author</th><th>Body</th></tr></thead><tbody>"
            + "".join(ex_rows) + "</tbody></table>"
        )
        cards.append(
            f"<div class='cat-card'>"
            f"<div class='cat-head'><span class='cat-label'>{esc(CATEGORY_LABEL[cat])}</span>"
            f"<span class='cat-count'>{len(rows)} comments ({pct:.1f}%)</span></div>"
            f"{tbl}</div>"
        )
    return f"""
<section id="s2">
  <h2>2. Noise filter forensic breakdown</h2>
  <p class="muted">For every <code>is_noise=1</code> row, the body was re-tested
  against the compiled regexes from <code>lib/noise_filter.py</code> in the
  same order as <code>is_noise()</code>. The first match wins.</p>
  {''.join(cards)}
</section>
"""


def render_section3(by_cat, data):
    # Left column: 8 representative noise comments — sample across categories
    left = []
    seen = 0
    for cat in CATEGORY_ORDER:
        if seen >= 8:
            break
        for r in by_cat.get(cat, []):
            if seen >= 8:
                break
            body = truncate_body(r["body"], 320)
            left.append(
                f"<div class='ex-card'>"
                f"<div class='ex-head'>{tier_pill(r['tier'])} "
                f"<span class='auth'>{esc(r['author'])}</span> "
                f"<span class='pill neutral'>{esc(cat)}</span></div>"
                f"<div class='ex-body'><code class='body'>{esc(body)}</code></div>"
                f"</div>"
            )
            seen += 1
    # Right column: at least one of each from 6 taxonomies, padded to 8
    right_seen = []
    taxes = list(data["substantive_samples_by_tax"].keys())
    # First pass: one per tax
    for tax in taxes:
        rows = data["substantive_samples_by_tax"][tax]
        if rows:
            right_seen.append(rows[0])
    # Pad to 8 from the next-best of each
    while len(right_seen) < 8:
        added = False
        for tax in taxes:
            if len(right_seen) >= 8:
                break
            rows = data["substantive_samples_by_tax"][tax]
            for r in rows[1:]:
                if r not in right_seen:
                    right_seen.append(r)
                    added = True
                    break
        if not added:
            break
    right_html = []
    for cid, author, tier, body, tax, rule, conf in right_seen:
        body_t = truncate_body(body, 280)
        rule_t = truncate_body(rule or "", 240)
        right_html.append(
            f"<div class='ex-card'>"
            f"<div class='ex-head'>{tier_pill(tier)} "
            f"<span class='auth'>{esc(author)}</span> "
            f"<span class='pill good'>{esc(tax)}</span>"
            f"<span class='pill neutral'>conf {conf:.2f}</span></div>"
            f"<div class='ex-rule'><strong>Rule:</strong> {esc(rule_t)}</div>"
            f"<div class='ex-body'><code class='body'>{esc(body_t)}</code></div>"
            f"</div>"
        )
    return f"""
<section id="s3">
  <h2>3. Side-by-side: noise vs substantive</h2>
  <div class="two-col">
    <div>
      <h3>Filtered as noise (8)</h3>
      {''.join(left)}
    </div>
    <div>
      <h3>Classified as substantive — conf ≥ 0.6 (8)</h3>
      {''.join(right_html)}
    </div>
  </div>
</section>
"""


def render_section4(data):
    # Group stylistic rows
    by_tax = defaultdict(list)
    for row in data["stylistic_rows"]:
        by_tax[row[4]].append(row)  # row[4] = taxonomy

    cid_in_md = data["cid_in_agents"]

    # Summary stats
    total_styl = sum(len(v) for v in by_tax.values())
    gap_count = 0
    author_counter = Counter()
    for rows in by_tax.values():
        for row in rows:
            cid, author, tier, area, tax, rule, conf, addressed, resolved, body = row
            if cid_in_md.get(cid, 0) == 0:
                gap_count += 1
            author_counter[author] += 1
    top_author, top_n = (author_counter.most_common(1)[0] if author_counter else ("—", 0))

    cards = []
    for tax in STYLISTIC:
        rows = by_tax.get(tax, [])
        if not rows:
            continue
        avg_conf = sum(r[6] for r in rows) / len(rows)
        in_md = sum(1 for r in rows if cid_in_md.get(r[0], 0) == 1)
        not_in_md = len(rows) - in_md

        body_rows = []
        for row in rows:
            cid, author, tier, area, tax_, rule, conf, addressed, resolved, body = row
            covered = cid_in_md.get(cid, 0)
            gap_badge = (
                "<span class='pill bad'>gap</span>" if not covered
                else "<span class='pill good'>covered</span>"
            )
            addr_str = "—" if addressed is None else ("yes" if addressed else "no")
            res_str = "yes" if resolved else "no"
            rule_t = esc(truncate_body(rule or "", 280))
            body_rows.append(
                f"<tr class='{'row-gap' if not covered else ''}'>"
                f"<td class='num'>{cid}</td>"
                f"<td>{esc(author)}</td>"
                f"<td>{tier_pill(tier)}</td>"
                f"<td>{esc(area)}</td>"
                f"<td class='rule-cell'>{rule_t}</td>"
                f"<td class='num'>{conf:.2f}</td>"
                f"<td>{addr_str}</td>"
                f"<td>{res_str}</td>"
                f"<td>{gap_badge}</td>"
                f"</tr>"
            )
        tbl = (
            "<table class='tbl dense'><thead><tr>"
            "<th>id</th><th>Author</th><th>Tier</th><th>Area</th>"
            "<th>rule_statement</th><th>conf</th><th>addressed</th>"
            "<th>resolved</th><th>coverage</th>"
            "</tr></thead><tbody>" + "".join(body_rows) + "</tbody></table>"
        )
        cards.append(
            f"<div class='styl-card'>"
            f"<div class='styl-head'>"
            f"<h3 class='styl-title'>{esc(tax)}</h3>"
            f"<span class='pill neutral'>{len(rows)} rows</span>"
            f"<span class='pill neutral'>avg conf {avg_conf:.2f}</span>"
            f"<span class='pill good'>{in_md} covered</span>"
            f"<span class='pill bad'>{not_in_md} gap candidates</span>"
            f"</div>{tbl}</div>"
        )

    summary = f"""
    <div class="summary-box">
      <p><strong>{total_styl}</strong> stylistic rule classifications across {len([t for t in STYLISTIC if by_tax.get(t)])} categories.</p>
      <p><strong>{gap_count}</strong> of them have no covering finding flagged <code>in_agents_md=1</code> — strong candidates for a dedicated "stylistic Rust conventions" skill output.</p>
      <p>Top stylistic reviewer: <strong>{esc(top_author)}</strong> with {top_n} stylistic comments.</p>
    </div>
    """

    return f"""
<section id="s4">
  <h2>4. Stylistic Rust convention deep-dive</h2>
  {summary}
  {''.join(cards)}
</section>
"""


def render_section5(data, by_cat):
    # 5a: tier table (reuse)
    tier_rows = []
    for t, total, noise in data["tier_counts"]:
        noise = noise or 0
        rate = (noise / total * 100) if total else 0
        # Red if T1/T2 rate > 8%, amber > 5%, green otherwise
        cls = "good"
        if t in (1, 2):
            if rate > 8:
                cls = "bad"
            elif rate > 5:
                cls = "warn"
        tier_rows.append(
            f"<tr><td>{tier_pill(t)}</td>"
            f"<td class='num'>{total:,}</td>"
            f"<td class='num'>{noise:,}</td>"
            f"<td class='num'><span class='pill {cls}'>{rate:.1f}%</span></td></tr>"
        )
    tier_html = (
        "<table class='tbl'><thead><tr>"
        "<th>Tier</th><th>Total</th><th>Filtered</th><th>Noise rate</th>"
        "</tr></thead><tbody>" + "".join(tier_rows) + "</tbody></table>"
    )

    # 5b: borderline punts
    punt_rows = []
    for cid, author, tier, body, rule, conf in data["borderline_punts"]:
        body_t = truncate_body(body, 240)
        rule_t = esc(truncate_body(rule or "", 120))
        punt_rows.append(
            f"<tr>"
            f"<td class='num'>{cid}</td>"
            f"<td>{tier_pill(tier)} {esc(author)}</td>"
            f"<td class='body-cell'><code class='body'>{esc(body_t)}</code></td>"
            f"<td>{rule_t}</td>"
            f"<td class='num'>{conf:.2f}</td></tr>"
        )
    punt_html = (
        "<table class='tbl'><thead><tr>"
        "<th>id</th><th>Author</th><th>Body</th>"
        "<th>Classifier rule</th><th>conf</th>"
        "</tr></thead><tbody>" + "".join(punt_rows) + "</tbody></table>"
    )

    # 5c: at-risk praise/nit matches (body len 30-50)
    risk_rows = []
    risk_count = 0
    for r in by_cat.get("praise-nit-only", []):
        blen = len(r["body"] or "")
        if 30 <= blen <= 50:
            risk_count += 1
            body_t = truncate_body(r["body"], 200)
            risk_rows.append(
                f"<tr>"
                f"<td class='num'>{blen}</td>"
                f"<td>{tier_pill(r['tier'])} {esc(r['author'])}</td>"
                f"<td class='body-cell'><code class='body'>{esc(body_t)}</code></td>"
                f"</tr>"
            )
    risk_html = (
        "<table class='tbl'><thead><tr>"
        "<th>Body len</th><th>Author</th><th>Body</th>"
        "</tr></thead><tbody>" + "".join(risk_rows) + "</tbody></table>"
        if risk_rows else "<p class='muted'>No praise/nit matches with body length 30–50 chars.</p>"
    )

    return f"""
<section id="s5">
  <h2>5. Sanity check / borderline cases</h2>
  <h3>5a. Noise rate by reviewer tier</h3>
  <p class="muted">A spike in T1 filter rate would suggest the filter is dropping rules from the people most likely to teach them.</p>
  {tier_html}

  <h3>5b. Classifier-side punts ({len(data['borderline_punts'])} shown)</h3>
  <p class="muted">Comments the noise filter let through, but the classifier still tagged <code>taxonomy='other'</code> with <code>confidence &lt; 0.3</code>.
  These are candidates for filter tightening.</p>
  {punt_html}

  <h3>5c. At-risk praise/nit matches — body length 30–50 chars ({risk_count} rows)</h3>
  <p class="muted">Bodies that hug the praise/nit regex's 40-char post-prefix cap. Substantive nits in this range
  would indicate the cap is too loose.</p>
  {risk_html}
</section>
"""


def render_section6(data, by_cat, stylistic_summary):
    # Compute the recommendation inputs from the data.
    t1_total = next((tot for t, tot, _ in data["tier_counts"] if t == 1), 0)
    t1_noise = next((n or 0 for t, _, n in data["tier_counts"] if t == 1), 0)
    t1_rate = (t1_noise / t1_total * 100) if t1_total else 0

    risk_count = sum(
        1 for r in by_cat.get("praise-nit-only", [])
        if 30 <= len(r["body"] or "") <= 50
    )
    praise_total = len(by_cat.get("praise-nit-only", []))

    # 6a recommendation logic
    if risk_count <= 2 and t1_rate < 8:
        cal_label = "Keep as-is"
        cal_cls = "good"
        cal_body = (
            f"T1 noise rate is <strong>{t1_rate:.1f}%</strong> and only "
            f"<strong>{risk_count}</strong> praise/nit matches sit in the 30–50 char borderline zone. "
            f"The filter is well-calibrated for the current corpus. A future revisit makes sense once "
            f"the corpus doubles in size."
        )
    elif risk_count >= 5 or t1_rate > 12:
        cal_label = "Loosen"
        cal_cls = "bad"
        cal_body = (
            f"T1 noise rate is <strong>{t1_rate:.1f}%</strong> and <strong>{risk_count}</strong> "
            f"borderline praise/nit matches were found. Recommend dropping the praise/nit regex "
            f"entirely or shrinking the body cap from 40 → 20 chars."
        )
    else:
        cal_label = "Tighten cap"
        cal_cls = "warn"
        cal_body = (
            f"T1 noise rate is <strong>{t1_rate:.1f}%</strong> and <strong>{risk_count}</strong> "
            f"borderline praise/nit matches were found in the 30–50 char zone. Recommend lowering the "
            f"praise/nit body cap from 40 → 20 chars, which would re-route the borderline cases through "
            f"the classifier."
        )

    # 6b: stylistic skill recommendation
    gap_count = stylistic_summary["gap_count"]
    total_styl = stylistic_summary["total"]
    by_tax_gap = stylistic_summary["by_tax_gap"]
    top_gap_tax = max(by_tax_gap.items(), key=lambda kv: kv[1])[0] if by_tax_gap else "naming"

    if gap_count > 30:
        styl_label = "Extract a dedicated stylistic-Rust skill"
        styl_cls = "good"
    else:
        styl_label = "Roll into existing skill (gap too small)"
        styl_cls = "warn"

    # Top 5–10 gap rule_statements (deduped, by confidence desc within gaps)
    cid_in_md = data["cid_in_agents"]
    gap_rules = []
    seen_rules = set()
    for row in sorted(data["stylistic_rows"], key=lambda r: -r[6]):
        cid, author, tier, area, tax, rule, conf, *_ = row
        if cid_in_md.get(cid, 0) == 1:
            continue
        rule_clean = (rule or "").strip()
        if not rule_clean:
            continue
        key = rule_clean.lower()[:80]
        if key in seen_rules:
            continue
        seen_rules.add(key)
        gap_rules.append((tax, rule_clean, conf, author))
        if len(gap_rules) >= 10:
            break

    gap_rule_html = "".join(
        f"<li><span class='pill neutral'>{esc(tax)}</span> "
        f"<span class='pill neutral'>conf {conf:.2f}</span> "
        f"<span class='pill neutral'>{esc(author)}</span> "
        f"{esc(truncate_body(rule, 260))}</li>"
        for tax, rule, conf, author in gap_rules
    )

    # 6c: taxonomy splits — heuristic clustering based on rule_statement keywords
    naming_rules = [
        r for r in data["stylistic_rows"] if r[4] == "naming" and r[5]
    ]
    domain_naming = sum(
        1 for r in naming_rules
        if any(k in (r[5] or "").lower() for k in [
            "url", "type", "domain", "use ", "instead of string", "parse", "newtype"
        ])
    )
    convention_naming = len(naming_rules) - domain_naming
    cf_rules = [r for r in data["stylistic_rows"] if r[4] == "control-flow" and r[5]]
    early_return = sum(1 for r in cf_rules if any(k in (r[5] or "").lower() for k in ["early return", "guard", "return"]))
    nested = sum(1 for r in cf_rules if any(k in (r[5] or "").lower() for k in ["nested", "decompos", "extract", "helper"]))
    match_style = sum(1 for r in cf_rules if any(k in (r[5] or "").lower() for k in ["match", "if let", "let else"]))

    return f"""
<section id="s6">
  <h2>6. Recommendations</h2>

  <h3>6a. Noise filter calibration</h3>
  <div class="rec-card {cal_cls}">
    <div class="rec-label">Recommendation: {cal_label}</div>
    <p>{cal_body}</p>
    <p class="muted">Inputs — T1 rate: {t1_rate:.1f}% · praise/nit total: {praise_total} · borderline (30–50 char): {risk_count}</p>
  </div>

  <h3>6b. Stylistic skill recommendation</h3>
  <div class="rec-card {styl_cls}">
    <div class="rec-label">Recommendation: {styl_label}</div>
    <p><strong>{gap_count}</strong> of <strong>{total_styl}</strong> stylistic rule classifications are not yet
    covered by an <code>in_agents_md=1</code> finding. Strongest gap signal comes from <strong>{esc(top_gap_tax)}</strong>
    ({by_tax_gap.get(top_gap_tax, 0)} gap rows).</p>
    <p>Top gap-candidate rule statements (deduped, sorted by classifier confidence):</p>
    <ol class="rule-list">{gap_rule_html}</ol>
  </div>

  <h3>6c. Possible taxonomy splits</h3>
  <div class="rec-card neutral">
    <p><strong>naming</strong> → consider splitting into:</p>
    <ul>
      <li><code>domain-naming</code> (e.g., "use <code>Url</code> not <code>String</code>", parse-at-boundaries) — approx <strong>{domain_naming}</strong> rows match this pattern.</li>
      <li><code>convention-naming</code> (e.g., "no <code>test_</code> prefix on test functions", helper naming) — approx <strong>{convention_naming}</strong> rows.</li>
    </ul>
    <p><strong>control-flow</strong> → consider splitting into:</p>
    <ul>
      <li><code>early-return</code> / guard-style — approx <strong>{early_return}</strong> rows.</li>
      <li><code>function-decomposition</code> (extract nested logic into helpers) — approx <strong>{nested}</strong> rows.</li>
      <li><code>match-style</code> (<code>match</code> vs <code>if let</code> vs <code>let else</code>) — approx <strong>{match_style}</strong> rows.</li>
    </ul>
    <p class="muted">These counts are keyword-heuristic over <code>rule_statement</code>; treat as a triage signal, not a final partition.</p>
  </div>
</section>
"""


def render_footer(data):
    return f"""
<section id="footer">
  <h2>7. Footer</h2>
  <p class="muted">DB snapshot mtime: <code>{esc(data['db_mtime'])}</code></p>
  <p class="muted">Companion reports:
    <a href="rust-pr-analysis-dashboard-01.html">main dashboard</a> ·
    <a href="rust-pr-analysis-jouney-01.html">journey log</a>
  </p>
  <p class="muted">Built by <code>scripts/pr-analysis/build_noise_deep_dive.py</code>.
  This report audits — and does not modify — <code>lib/noise_filter.py</code>.</p>
</section>
"""


# ──────────────────────────────────────────────────────────────────────────
# CSS
# ──────────────────────────────────────────────────────────────────────────

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
.container { max-width: 1240px; margin: 0 auto; }
header.page-header {
  background: var(--bg-card); border: 1px solid var(--border);
  border-radius: 8px; padding: 24px 28px; box-shadow: var(--shadow);
  margin-bottom: 22px;
}
header.page-header h1 { margin: 0 0 4px; font-size: 24px; font-weight: 600; letter-spacing: -.01em; }
header.page-header .subtitle { color: var(--fg-mute); font-size: 14px; }
section {
  background: var(--bg-card); border: 1px solid var(--border);
  border-radius: 8px; padding: 22px 26px; margin-bottom: 22px;
  box-shadow: var(--shadow);
}
section h2 { margin: 0 0 12px; font-size: 19px; font-weight: 600; letter-spacing: -.005em; }
section h3 { margin: 18px 0 8px; font-size: 13px; font-weight: 600; color: var(--fg-mute); text-transform: uppercase; letter-spacing: .04em; }
.stat-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 12px; }
.stat { background: var(--code-bg); border-radius: 6px; padding: 10px 12px; }
.stat .label { font-size: 11px; text-transform: uppercase; letter-spacing: .04em; color: var(--fg-mute); }
.stat .value { font-size: 17px; font-weight: 600; font-variant-numeric: tabular-nums; margin-top: 2px; }
p { margin: 0 0 10px; }
p.muted { color: var(--fg-mute); font-size: 13px; }
code { font-family: "SF Mono", Menlo, Consolas, monospace; font-size: 12.5px; background: var(--code-bg); padding: 1px 5px; border-radius: 3px; }
code.body { display: block; white-space: pre-wrap; word-break: break-word; padding: 6px 8px; background: var(--code-bg); }
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

table.tbl { border-collapse: collapse; font-size: 13px; width: 100%; }
table.tbl th, table.tbl td { border: 1px solid var(--border); padding: 6px 10px; text-align: left; vertical-align: top; }
table.tbl th { background: var(--code-bg); font-weight: 600; color: var(--fg-mute); text-transform: uppercase; font-size: 11px; letter-spacing: .04em; }
table.tbl td.num { font-variant-numeric: tabular-nums; text-align: right; }
table.tbl.examples td.body-cell { width: 80%; }
table.tbl.dense th, table.tbl.dense td { padding: 4px 8px; font-size: 12px; }
table.tbl .rule-cell { max-width: 480px; }
tr.row-gap td { background: #fff8f8; }

.cat-card { border: 1px solid var(--border); border-radius: 6px; padding: 12px 14px; margin-bottom: 14px; }
.cat-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
.cat-label { font-weight: 600; font-size: 14px; }
.cat-count { font-size: 12px; color: var(--fg-mute); font-variant-numeric: tabular-nums; }

.two-col { display: grid; grid-template-columns: 1fr 1fr; gap: 18px; }
@media (max-width: 1024px) { .two-col { grid-template-columns: 1fr; } }
.ex-card { border: 1px solid var(--border); border-radius: 6px; padding: 10px 12px; margin-bottom: 10px; background: var(--bg-card); }
.ex-head { font-size: 12px; margin-bottom: 6px; display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
.ex-rule { font-size: 13px; margin-bottom: 6px; }
.ex-body code.body { font-size: 12px; }
.auth { font-weight: 600; }

.styl-card { border: 1px solid var(--border); border-radius: 6px; padding: 12px 14px; margin-bottom: 18px; }
.styl-head { display: flex; gap: 8px; align-items: center; margin-bottom: 8px; flex-wrap: wrap; }
.styl-title { margin: 0; text-transform: none; color: var(--fg); font-size: 15px; letter-spacing: 0; }

.summary-box { background: var(--code-bg); border-radius: 6px; padding: 12px 16px; margin-bottom: 16px; border-left: 3px solid var(--accent); }
.summary-box p { margin-bottom: 4px; font-size: 14px; }

.rec-card { border: 1px solid var(--border); border-radius: 6px; padding: 12px 14px; margin-bottom: 14px; }
.rec-card.good { background: #f3faf5; border-left: 3px solid var(--good); }
.rec-card.warn { background: #fdf7e9; border-left: 3px solid var(--warn); }
.rec-card.bad  { background: #fdf3f3; border-left: 3px solid var(--bad); }
.rec-card.neutral { background: var(--code-bg); }
.rec-label { font-weight: 600; margin-bottom: 6px; font-size: 14px; }
.rule-list { padding-left: 22px; font-size: 13px; }
.rule-list li { margin-bottom: 6px; }
"""


# ──────────────────────────────────────────────────────────────────────────
# Render top-level
# ──────────────────────────────────────────────────────────────────────────

def render(data, by_cat, stylistic_summary):
    noise_pct = data["lc_noise"] / data["lc_total"] * 100 if data["lc_total"] else 0
    page = f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Flox PR-analysis — noise filter audit & stylistic deep-dive</title>
<style>{CSS}</style>
</head>
<body>
<div class="container">
  <header class="page-header">
    <h1>Flox PR-analysis — noise filter audit & stylistic Rust convention deep-dive</h1>
    <div class="subtitle">
      Forensic per-regex breakdown of the {data['lc_noise']:,} filtered comments
      ({noise_pct:.1f}% of {data['lc_total']:,}), plus a full audit of the five
      stylistic taxonomies and their AGENTS.md coverage gaps.
    </div>
  </header>
  {render_section1(data, by_cat)}
  {render_section2(by_cat)}
  {render_section3(by_cat, data)}
  {render_section4(data)}
  {render_section5(data, by_cat)}
  {render_section6(data, by_cat, stylistic_summary)}
  {render_footer(data)}
</div>
</body>
</html>
"""
    return page


def main():
    conn = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    data = fetch(conn)

    # Bucket noise rows by which regex caught them
    by_cat = defaultdict(list)
    for cid, pr, author, tier, body in data["noise_rows"]:
        cat = which_regex(body or "")
        by_cat[cat].append({
            "id": cid, "pr": pr, "author": author,
            "tier": tier, "body": body or "",
        })

    # Stylistic summary for sections 4 & 6
    cid_in_md = data["cid_in_agents"]
    by_tax_gap = defaultdict(int)
    gap_count = 0
    for row in data["stylistic_rows"]:
        cid = row[0]
        tax = row[4]
        if cid_in_md.get(cid, 0) == 0:
            gap_count += 1
            by_tax_gap[tax] += 1
    stylistic_summary = {
        "total": len(data["stylistic_rows"]),
        "gap_count": gap_count,
        "by_tax_gap": dict(by_tax_gap),
    }

    html_out = render(data, by_cat, stylistic_summary)
    OUT.write_text(html_out, encoding="utf-8")

    # Stdout summary for the operator
    print(f"Wrote {OUT}")
    print(f"  size: {OUT.stat().st_size:,} bytes")
    print(f"  total line-comments: {data['lc_total']:,}")
    print(f"  filtered as noise:   {data['lc_noise']:,}")
    print("  regex breakdown:")
    for cat in CATEGORY_ORDER:
        n = len(by_cat.get(cat, []))
        if n:
            print(f"    {cat:>18s}: {n}")
    print("  tier noise rates:")
    for t, total, noise in data["tier_counts"]:
        noise = noise or 0
        rate = (noise / total * 100) if total else 0
        print(f"    T{t}: {noise}/{total} ({rate:.1f}%)")
    print(f"  stylistic rows: {stylistic_summary['total']} · gap: {stylistic_summary['gap_count']}")


if __name__ == "__main__":
    main()
