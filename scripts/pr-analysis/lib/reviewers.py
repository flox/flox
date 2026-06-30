"""Reviewer tiering and weighting derived from 6-month volume analysis."""
from __future__ import annotations

from dataclasses import dataclass

TIER1 = ("ysndr", "mkenigs", "dcarley")
TIER2 = ("djsauble", "gilmishal", "billlevine")

WEIGHTS = {1: 3.0, 2: 2.0, 3: 1.0, 4: 0.0}


@dataclass(frozen=True)
class Reviewer:
    login: str
    tier: int
    weight: float


def classify(login: str, author_type: str) -> Reviewer:
    if author_type == "Bot":
        return Reviewer(login, 4, WEIGHTS[4])
    if login in TIER1:
        return Reviewer(login, 1, WEIGHTS[1])
    if login in TIER2:
        return Reviewer(login, 2, WEIGHTS[2])
    return Reviewer(login, 3, WEIGHTS[3])


def seed_rows() -> list[tuple[str, float, int, str]]:
    rows = []
    for login in TIER1:
        rows.append((login, WEIGHTS[1], 1, "tier1: top opinionated reviewer"))
    for login in TIER2:
        rows.append((login, WEIGHTS[2], 2, "tier2: top-6 reviewer"))
    return rows
