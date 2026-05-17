"""Taxonomy seeded from AGENTS.md sections.

Each entry has a short id, a description used in LLM prompts, and a list
of AGENTS.md section anchors used by the gap-report to mark coverage.
"""
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class TaxonomyEntry:
    id: str
    description: str
    agents_md_sections: tuple[str, ...]


TAXONOMY: tuple[TaxonomyEntry, ...] = (
    TaxonomyEntry(
        "error-handling",
        "Error type hierarchy, classification, where to extend enums vs. parse strings, "
        "credential sanitization, message rewriting at product boundary.",
        ("Error handling architecture",),
    ),
    TaxonomyEntry(
        "provider-traits",
        "When to introduce a provider trait vs. concrete type, associated-type usage, "
        "consumer constraints.",
        ("Provider traits and associated types",),
    ),
    TaxonomyEntry(
        "type-safety",
        "Parsing at boundaries, preferring domain types (Url, PackageSystem, NixFlakeref, "
        "BaseCatalogUrl) over raw strings.",
        ("Type safety at function boundaries",),
    ),
    TaxonomyEntry(
        "user-facing-messages",
        "Sentence structure, brand naming, emoji map, next-step suggestions, line wrap, voice.",
        ("User-visible message syntax, structure, and content", "CLI output conventions"),
    ),
    TaxonomyEntry(
        "naming",
        "Helper naming conventions, test naming (no test_ prefix), descriptive names.",
        ("Naming new helpers", "Test naming"),
    ),
    TaxonomyEntry(
        "testing",
        "assert_eq on entire structs, integration vs unit, bats filter patterns, test data.",
        ("Rust style",),
    ),
    TaxonomyEntry(
        "imports",
        "Import organization: use vs. ::-qualification, where to place use statements "
        "(module top vs. nearest function), updating use when moving code, re-export "
        "discipline, import grouping (stdlib/external/local).",
        ("use guidelines",),
    ),
    TaxonomyEntry(
        "manifest-usage",
        "Manifest<S> type-state, never serialize by hand, schema-version rules.",
        ("Manifest usage",),
    ),
    TaxonomyEntry(
        "deprecated-patterns",
        "Recognising and avoiding deprecated infrastructure, unimplemented!() over extending.",
        ("Deprecated infrastructure",),
    ),
    TaxonomyEntry(
        "logging-tracing",
        "Structured tracing fields rather than interpolation.",
        ("Rust style",),
    ),
    TaxonomyEntry(
        "formatting-style",
        "formatdoc!/indoc!, single-quote command examples, line length on user strings.",
        ("Rust style",),
    ),
    TaxonomyEntry(
        "control-flow",
        "Early returns, functional style, avoid nested conditionals, Clone/Debug derives.",
        ("Rust style",),
    ),
    TaxonomyEntry(
        "semantic-correctness",
        "Understand semantics before rewriting messages or refactoring.",
        ("Understand semantics before rewriting messages",),
    ),
    TaxonomyEntry(
        "ld-floxlib",
        "GLIBC version binding rules for ld-floxlib.c.",
        ("ld-floxlib",),
    ),
    TaxonomyEntry(
        "panic-discipline",
        "Avoid panics in library code; prefer Result<E> or explicit error handling. "
        "Includes guidance against unwrap()/expect() on user-facing paths and against "
        "panic-on-startup patterns.",
        ("Error handling architecture",),
    ),
    TaxonomyEntry(
        "other",
        "Anything that does not fit; open bucket for new rules.",
        (),
    ),
)

TAXONOMY_IDS: tuple[str, ...] = tuple(t.id for t in TAXONOMY)
TAXONOMY_BY_ID = {t.id: t for t in TAXONOMY}
