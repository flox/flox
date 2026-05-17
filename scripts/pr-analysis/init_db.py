#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Initialize the SQLite database: apply schema and seed reviewer rows.

Pass --reset to drop the existing DB file first. Used between the pilot
run (Task 7) and the full-corpus run (Task 8), and any time a schema
revision lands during the retro loop.
"""
from __future__ import annotations

import argparse
import datetime as dt
from pathlib import Path

from lib.db import DEFAULT_DB_PATH, apply_schema, connect, transaction
from lib.reviewers import seed_rows

SCHEMA_PATH = Path(__file__).resolve().parent / "schema.sql"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reset", action="store_true",
                        help="delete the existing DB file before applying schema")
    args = parser.parse_args()

    if args.reset and DEFAULT_DB_PATH.exists():
        DEFAULT_DB_PATH.unlink()
        for suffix in ("-wal", "-shm", "-journal"):
            sidecar = DEFAULT_DB_PATH.with_name(DEFAULT_DB_PATH.name + suffix)
            if sidecar.exists():
                sidecar.unlink()
        print(f"reset: removed {DEFAULT_DB_PATH}")

    conn = connect()
    apply_schema(conn, SCHEMA_PATH)
    with transaction(conn):
        conn.executemany(
            "INSERT OR REPLACE INTO reviewer (login, weight, tier, notes) "
            "VALUES (?, ?, ?, ?)",
            seed_rows(),
        )
    print(f"initialized db; reviewers seeded; now: {dt.datetime.now(dt.UTC).isoformat()}")


if __name__ == "__main__":
    main()
