#!/usr/bin/env python3
"""Report Alpha SQLite storage usage without opening the database for writes."""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
from pathlib import Path
from typing import Any


DEFAULT_CAPS = {
    "alpha_observation_events": 50_000,
    "alpha_paper_signals": 20_000,
}

TEXT_COLUMNS = {
    "alpha_observation_events": ("raw_json",),
    "alpha_paper_signals": ("features_json",),
}

TIME_COLUMNS = {
    "alpha_observation_events": "received_at_ms",
    "alpha_paper_signals": "created_at_ms",
}


def connect_readonly(path: Path) -> sqlite3.Connection:
    if not path.is_file():
        raise SystemExit(f"database does not exist: {path}")
    connection = sqlite3.connect(f"file:{path.resolve()}?mode=ro", uri=True)
    connection.execute("PRAGMA query_only=ON")
    return connection


def table_exists(connection: sqlite3.Connection, table: str) -> bool:
    return (
        connection.execute(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (table,)
        ).fetchone()
        is not None
    )


def table_report(
    connection: sqlite3.Connection, table: str, row_cap: int
) -> dict[str, Any]:
    if not table_exists(connection, table):
        return {"present": False, "row_cap": row_cap}

    count = int(connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0])
    payload_bytes = {
        column: int(
            connection.execute(
                f"SELECT COALESCE(SUM(length({column})), 0) FROM {table}"
            ).fetchone()[0]
        )
        for column in TEXT_COLUMNS[table]
    }
    time_column = TIME_COLUMNS[table]
    oldest, newest = connection.execute(
        f"SELECT MIN({time_column}), MAX({time_column}) FROM {table}"
    ).fetchone()
    report: dict[str, Any] = {
        "present": True,
        "rows": count,
        "row_cap": row_cap,
        "over_row_cap": count > row_cap,
        "at_row_cap": count >= row_cap,
        "capacity_remaining_rows": max(row_cap - count, 0),
        "oldest_timestamp_ms": oldest,
        "newest_timestamp_ms": newest,
        "payload_bytes": payload_bytes,
        "payload_bytes_total": sum(payload_bytes.values()),
    }
    if table == "alpha_paper_signals":
        report["protected_rows"] = int(
            connection.execute(
                "SELECT COUNT(*) FROM alpha_paper_signals WHERE COALESCE(status, '') NOT IN "
                "('paper', 'simulated', 'confirmed', 'failed', 'blocked', 'cancelled', 'expired')"
            ).fetchone()[0]
        )
    return report


def file_sizes(path: Path) -> dict[str, int]:
    sizes = {"database": path.stat().st_size}
    for suffix in ("-wal", "-shm"):
        sidecar = Path(f"{path}{suffix}")
        sizes[suffix[1:]] = sidecar.stat().st_size if sidecar.exists() else 0
    sizes["total"] = sum(sizes.values())
    return sizes


def build_report(connection: sqlite3.Connection, path: Path, caps: dict[str, int]) -> dict[str, Any]:
    page_count, page_size = connection.execute(
        "SELECT page_count, page_size FROM pragma_page_count(), pragma_page_size()"
    ).fetchone()
    tables = {
        table: table_report(connection, table, caps[table]) for table in DEFAULT_CAPS
    }
    return {
        "database": str(path),
        "read_only": True,
        "file_bytes": file_sizes(path),
        "sqlite_page_count": int(page_count),
        "sqlite_page_size": int(page_size),
        "tables": tables,
        "warnings": [
            f"{table} {'exceeds' if report.get('over_row_cap') else 'reached'} row cap"
            for table, report in tables.items()
            if report.get("at_row_cap")
        ],
    }


def env_cap(name: str, default: int) -> int:
    value = os.environ.get(name)
    if value is None:
        return default
    try:
        parsed = int(value)
    except ValueError as error:
        raise SystemExit(f"{name} must be a positive integer") from error
    if parsed <= 0:
        raise SystemExit(f"{name} must be a positive integer")
    return parsed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path)
    parser.add_argument(
        "--observation-max-rows",
        type=int,
        default=env_cap("ALPHA_OBSERVATION_MAX_ROWS", DEFAULT_CAPS["alpha_observation_events"]),
    )
    parser.add_argument(
        "--paper-signal-max-rows",
        type=int,
        default=env_cap("ALPHA_PAPER_SIGNAL_MAX_ROWS", DEFAULT_CAPS["alpha_paper_signals"]),
    )
    args = parser.parse_args()
    caps = {
        "alpha_observation_events": args.observation_max_rows,
        "alpha_paper_signals": args.paper_signal_max_rows,
    }
    if any(value <= 0 for value in caps.values()):
        parser.error("row caps must be positive")
    connection = connect_readonly(args.database)
    try:
        print(json.dumps(build_report(connection, args.database, caps), ensure_ascii=False, indent=2))
    finally:
        connection.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
