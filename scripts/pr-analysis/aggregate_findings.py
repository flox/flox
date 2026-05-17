#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "sentence-transformers>=2.5",
# ]
# ///
"""Aggregate classified comments into `finding` rows.

Approach:
- Group by (taxonomy, area). Within each group, cluster rule_statements by
  cosine similarity of MiniLM sentence embeddings to merge near-duplicates.
- For each cluster: emit one finding row with reviewer attribution, evidence,
  cross-area count (number of areas in which this same theme also clustered),
  AGENTS.md coverage, and a confidence score.

Idempotent: deletes existing findings before re-aggregating.
"""
from __future__ import annotations

import datetime as dt
import json
import re
from collections import Counter, defaultdict
from pathlib import Path

from lib.areas import HOT_AREAS
from lib.db import connect, transaction
from lib.taxonomy import TAXONOMY_BY_ID

AGENTS_MD_PATH = Path(__file__).resolve().parent.parent.parent / "AGENTS.md"

CLUSTER_THRESHOLD = 0.65  # cosine similarity (was 0.35 for Jaccard)
EMBEDDING_MODEL = "all-MiniLM-L6-v2"


def _tokens(s: str) -> set[str]:
    return set(re.findall(r"[a-z0-9_]+", s.lower())) - {
        "the", "a", "an", "of", "to", "in", "on", "for", "is", "are", "and",
        "or", "with", "at", "by", "use", "uses", "used", "using",
    }


def _embedder():
    """Lazy-load the embedding model. Cached per process."""
    global _embedder_instance
    if "_embedder_instance" not in globals():
        from sentence_transformers import SentenceTransformer
        _embedder_instance = SentenceTransformer(EMBEDDING_MODEL)
    return _embedder_instance


def cluster_rule_statements(statements: list[str],
                            threshold: float = CLUSTER_THRESHOLD
                            ) -> list[list[int]]:
    """Greedy clustering by cosine similarity of MiniLM embeddings.

    For each statement: if its similarity to any existing cluster's mean
    embedding exceeds threshold, join that cluster; else start a new one.
    Statements with empty token sets are silently skipped.
    """
    import numpy as np

    # Mask out empty-token statements (mirror prior behavior)
    indices = [i for i, s in enumerate(statements) if _tokens(s)]
    if not indices:
        return []
    texts = [statements[i] for i in indices]
    embeddings = _embedder().encode(
        texts, normalize_embeddings=True, show_progress_bar=False
    )

    clusters: list[list[int]] = []
    cluster_means: list[np.ndarray] = []
    for local_idx, original_idx in enumerate(indices):
        vec = embeddings[local_idx]
        placed = False
        for c_idx, mean in enumerate(cluster_means):
            sim = float(np.dot(mean, vec))
            if sim >= threshold:
                clusters[c_idx].append(original_idx)
                # update mean (still normalized since we re-normalize)
                new_mean = (mean * (len(clusters[c_idx]) - 1) + vec) / len(clusters[c_idx])
                norm = np.linalg.norm(new_mean)
                cluster_means[c_idx] = new_mean / norm if norm > 0 else new_mean
                placed = True
                break
        if not placed:
            clusters.append([original_idx])
            cluster_means.append(vec)
    return clusters


def confidence_score(*, tier1_count: int, tier2_count: int,
                     total_evidence: int, cross_area_count: int,
                     acceptance_rate: float) -> float:
    tier_signal = 1.0 if tier1_count > 0 else (0.5 if tier2_count > 0 else 0.0)
    return round(
        0.4 * tier_signal
        + 0.2 * min(total_evidence / 5.0, 1.0)
        + 0.2 * min(cross_area_count / 3.0, 1.0)
        + 0.2 * (acceptance_rate if acceptance_rate is not None else 0.5),
        3,
    )


def determine_scope(*, tier1_count: int, cross_area_count: int) -> str:
    if tier1_count >= 1 and cross_area_count >= 2:
        return "cross-cutting"
    return "area-specific"


def agents_md_coverage(rule_statement: str, agents_text: str,
                      min_token_len: int = 4,
                      min_overlap: int = 3) -> tuple[int, str | None]:
    """Return (1, matched_section_title) if at least `min_overlap` distinctive
    tokens from rule_statement appear in the same AGENTS.md section.

    'Distinctive' = length >= min_token_len AND not in the stopword set.
    This is forgiving to phrasing differences (imperative rules vs descriptive
    prose) where Jaccard fails because vocabularies don't overlap enough.
    """
    rule_tokens = {
        t for t in _tokens(rule_statement)
        if len(t) >= min_token_len
    }
    if len(rule_tokens) < min_overlap:
        # Not enough distinctive tokens to make a reliable match.
        return (0, None)
    best_section: str | None = None
    best_overlap = 0
    section_blocks = re.split(r"\n(?=#{2,4} )", agents_text)
    for block in section_blocks:
        if not block.strip():
            continue
        heading_match = re.match(r"#{2,4} (.+)", block)
        title = heading_match.group(1).strip() if heading_match else "(intro)"
        body_tokens = _tokens(block)
        overlap = len(rule_tokens & body_tokens)
        if overlap > best_overlap:
            best_overlap = overlap
            best_section = title
    if best_overlap >= min_overlap:
        return (1, best_section)
    return (0, None)


def main() -> None:
    conn = connect()
    agents_text = AGENTS_MD_PATH.read_text() if AGENTS_MD_PATH.exists() else ""

    classified = conn.execute(
        """SELECT c.comment_id, c.taxonomy, c.was_addressed, c.rule_statement, c.confidence,
                  lc.area, lc.pr_number, lc.author, lc.reviewer_tier
           FROM classification c
           JOIN line_comment lc ON lc.id = c.comment_id
           WHERE c.rule_statement <> '' AND c.confidence >= 0.4"""
    ).fetchall()

    # First pass: cluster per (taxonomy, area) to know area-level themes,
    # then derive a global theme key per cluster so we can count cross-areas.
    by_tax_area: dict[tuple[str, str], list[dict]] = defaultdict(list)
    for r in classified:
        by_tax_area[(r["taxonomy"], r["area"])].append(dict(r))

    # area_themes[(tax, area)] = list of clusters; each cluster = list of comment dicts
    area_themes: dict[tuple[str, str], list[list[dict]]] = {}
    for key, rows in by_tax_area.items():
        statements = [r["rule_statement"] for r in rows]
        clusters_idx = cluster_rule_statements(statements)
        area_themes[key] = [[rows[i] for i in idxs] for idxs in clusters_idx]

    # Build a global theme signature -> list of (area, cluster) for cross-area count.
    def signature(cluster: list[dict]) -> frozenset[str]:
        # use the union of meaningful tokens across all statements as the signature
        toks: set[str] = set()
        for r in cluster:
            toks |= _tokens(r["rule_statement"])
        # keep top 8 tokens by length to stabilize
        return frozenset(sorted(toks, key=lambda t: -len(t))[:8])

    sig_to_areas: dict[frozenset[str], set[str]] = defaultdict(set)
    for (tax, area), clusters in area_themes.items():
        for cluster in clusters:
            sig_to_areas[signature(cluster)].add(area)

    with transaction(conn):
        conn.execute("DELETE FROM finding")
        for (tax, area), clusters in area_themes.items():
            for cluster in clusters:
                sig = signature(cluster)
                areas_seen = sorted(sig_to_areas[sig])
                cross_area_count = len(areas_seen)
                tier1 = len({r["author"] for r in cluster if r["reviewer_tier"] == 1})
                tier2 = len({r["author"] for r in cluster if r["reviewer_tier"] == 2})
                addressed = [r["was_addressed"] for r in cluster if r["was_addressed"] is not None]
                acceptance = (sum(addressed) / len(addressed)) if addressed else None
                # pick the canonical rule_statement = the longest one (most descriptive)
                canonical = max(cluster, key=lambda r: len(r["rule_statement"] or ""))["rule_statement"]
                # primary reviewer = most-frequent author in cluster
                top_author, _ = Counter(r["author"] for r in cluster).most_common(1)[0]
                in_md, section = agents_md_coverage(canonical, agents_text)
                conn.execute(
                    """INSERT INTO finding
                       (theme, rule_statement, taxonomy, area, scope,
                        primary_reviewer, reviewer_logins,
                        tier1_reviewer_count, tier2_reviewer_count,
                        total_evidence_count, evidence_comment_ids, evidence_pr_numbers,
                        cross_area_count, areas_seen, acceptance_rate,
                        in_agents_md, agents_md_section, confidence_score, notes,
                        created_at)
                       VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
                    (
                        canonical[:80] or tax,
                        canonical,
                        tax,
                        area,
                        determine_scope(tier1_count=tier1, cross_area_count=cross_area_count),
                        top_author,
                        json.dumps(sorted({r["author"] for r in cluster})),
                        tier1, tier2,
                        len(cluster),
                        json.dumps([r["comment_id"] for r in cluster]),
                        json.dumps(sorted({r["pr_number"] for r in cluster})),
                        cross_area_count,
                        json.dumps(areas_seen),
                        acceptance,
                        in_md, section,
                        confidence_score(
                            tier1_count=tier1, tier2_count=tier2,
                            total_evidence=len(cluster),
                            cross_area_count=cross_area_count,
                            acceptance_rate=acceptance if acceptance is not None else 0.5,
                        ),
                        None,
                        dt.datetime.now(dt.UTC).isoformat(),
                    ),
                )
    n = conn.execute("SELECT COUNT(*) AS n FROM finding").fetchone()["n"]
    print(f"wrote {n} findings")
    by_scope = conn.execute(
        "SELECT scope, COUNT(*) AS n FROM finding GROUP BY scope"
    ).fetchall()
    for r in by_scope:
        print(f"  {r['scope']}: {r['n']}")


if __name__ == "__main__":
    main()
