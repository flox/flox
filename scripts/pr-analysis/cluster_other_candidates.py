#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["sentence-transformers>=2.5"]
# ///
"""Cluster high-confidence 'other'-bucket classifications to surface
candidate new taxonomy entries.

Selects classifications where:
  taxonomy='other' AND rule_statement <> '' AND confidence >= 0.5

Embeds each rule_statement via MiniLM, clusters greedily at cosine
similarity >= 0.55, prints each cluster with its rule_statements + a
suggested category name (derived from the longest common token sequence).
"""
from __future__ import annotations

import re
from collections import Counter

import numpy as np
from sentence_transformers import SentenceTransformer

from lib.db import connect

THRESHOLD = 0.55


def main() -> None:
    conn = connect()
    rows = conn.execute(
        """SELECT c.comment_id, c.rule_statement, c.confidence, lc.area, lc.body
           FROM classification c
           JOIN line_comment lc ON lc.id = c.comment_id
           WHERE c.taxonomy = 'other'
             AND c.rule_statement <> ''
             AND c.confidence >= 0.5
           ORDER BY c.confidence DESC"""
    ).fetchall()
    print(f"loaded {len(rows)} high-confidence 'other'-bucket rules\n")
    if not rows:
        return

    model = SentenceTransformer("all-MiniLM-L6-v2")
    statements = [r["rule_statement"] for r in rows]
    embeddings = model.encode(
        statements, normalize_embeddings=True, show_progress_bar=False
    )

    clusters: list[list[int]] = []
    cluster_means: list[np.ndarray] = []
    for i, vec in enumerate(embeddings):
        placed = False
        for j, mean in enumerate(cluster_means):
            if float(np.dot(mean, vec)) >= THRESHOLD:
                clusters[j].append(i)
                new_mean = (mean * (len(clusters[j]) - 1) + vec) / len(clusters[j])
                norm = np.linalg.norm(new_mean)
                cluster_means[j] = new_mean / norm if norm > 0 else new_mean
                placed = True
                break
        if not placed:
            clusters.append([i])
            cluster_means.append(vec)

    # Sort clusters by size desc
    clusters.sort(key=len, reverse=True)
    for k, cluster in enumerate(clusters, start=1):
        toks: Counter[str] = Counter()
        for idx in cluster:
            for t in re.findall(r"[a-z]{4,}", statements[idx].lower()):
                toks[t] += 1
        common = [t for t, n in toks.most_common(5) if n > 1]
        print(f"== Cluster {k}  size={len(cluster)}  common-tokens={common}")
        for idx in cluster:
            r = rows[idx]
            print(
                f"  [conf={r['confidence']:.2f} area={r['area']:18s}] "
                f"{r['rule_statement']}"
            )
        print()


if __name__ == "__main__":
    main()
